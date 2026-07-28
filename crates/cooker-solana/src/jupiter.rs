//! Unsigned Jupiter planning, strict response validation, and local v0 assembly.
//!
//! Jupiter is treated only as an off-chain instruction planner. This module deliberately has no
//! model or method for Jupiter's serialized-transaction, execute, or submission APIs. Validated
//! instructions are compiled with a Surfpool blockhash and signed locally.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    str::FromStr,
    sync::Arc,
    time::Duration,
};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use cooker_core::{
    ActionAdapter, ActionPayload, AdapterContext, ChainReceipt, ConfirmationStatus, CookerError,
    PlannedAction, PreparedAction, StateExpectation,
};
use reqwest::{Client, redirect::Policy as RedirectPolicy};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use solana_address_lookup_table_interface::{
    program as lookup_table_program, state::AddressLookupTable,
};
use solana_instruction::{AccountMeta, Instruction};
use solana_message::AddressLookupTableAccount;
use solana_program_pack::Pack;
use solana_pubkey::Pubkey;
use solana_signature::Signature;
use solana_transaction::versioned::VersionedTransaction;
use spl_associated_token_account_interface::address::get_associated_token_address;
use spl_token_interface::state::Account as TokenAccount;
use zeroize::Zeroize;

use crate::{
    LocalKeypair, SolanaGateway,
    adapter_support::{observe_until_terminal, parse_pubkey},
    build_signed_v0_transaction,
};

const JUPITER_API_BASE: &str = "https://api.jup.ag/swap/v1";
const JUPITER_LITE_API_BASE: &str = "https://lite-api.jup.ag/swap/v1";
const JUPITER_V6_PROGRAM: &str = "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4";
const RAYDIUM_CLMM_PROGRAM: &str = "CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK";
const COMPUTE_BUDGET_PROGRAM: &str = "ComputeBudget111111111111111111111111111111";
const SYSTEM_PROGRAM: &str = "11111111111111111111111111111111";
const TOKEN_PROGRAM: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
const ASSOCIATED_TOKEN_PROGRAM: &str = "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL";
const MAX_HTTP_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const MAX_ROUTE_STEPS: usize = 8;
const MAX_INSTRUCTIONS: usize = 16;
const MAX_ACCOUNTS_PER_INSTRUCTION: usize = 64;
const MAX_INSTRUCTION_DATA_BYTES: usize = 2_048;
const MAX_LOOKUP_TABLES: usize = 8;
const WRAPPED_SOL_MINT: &str = "So11111111111111111111111111111111111111112";
const JUPITER_INPUT_KIND: &str = "jupiter_input_token_delta";
const JUPITER_OUTPUT_KIND: &str = "jupiter_output_token_delta";
const JUPITER_ROUTE_KIND: &str = "jupiter_route_metadata";

/// Exact-input quote returned by Jupiter Swap v1.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JupiterQuote {
    /// Input token mint.
    pub input_mint: String,
    /// Exact raw input amount, encoded as a decimal string by Jupiter.
    pub in_amount: String,
    /// Output token mint.
    pub output_mint: String,
    /// Quoted raw output amount before slippage.
    pub out_amount: String,
    /// Minimum raw output amount enforced by the route.
    pub other_amount_threshold: String,
    /// Jupiter swap mode. Only `ExactIn` is accepted by this module.
    pub swap_mode: String,
    /// Slippage encoded in basis points.
    pub slippage_bps: u16,
    /// Decimal percentage price impact, retained as a string for exact comparison.
    pub price_impact_pct: String,
    /// Optional platform fee. Only absent or exactly zero is accepted.
    #[serde(default)]
    pub platform_fee: Option<JupiterPlatformFee>,
    /// Ordered route plan selected by Jupiter.
    pub route_plan: Vec<JupiterRouteStep>,
    /// Mainnet context slot used by the planner.
    pub context_slot: u64,
    /// Planner duration in seconds.
    pub time_taken: f64,
    /// Quote schema requested from Jupiter. Cooker only accepts the V2 instruction schema.
    pub instruction_version: String,
    /// Additive planner metadata retained verbatim when the quote is posted back to Jupiter.
    #[serde(flatten)]
    pub additional_fields: BTreeMap<String, Value>,
}

/// Platform fee embedded in a Jupiter quote.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JupiterPlatformFee {
    /// Raw fee amount.
    pub amount: String,
    /// Fee rate in basis points.
    pub fee_bps: u16,
}

/// One leg of a Jupiter route plan.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JupiterRouteStep {
    /// AMM and amount details for the leg.
    pub swap_info: JupiterSwapInfo,
    /// Legacy whole-percent split, when returned.
    #[serde(default)]
    pub percent: Option<u16>,
    /// Basis-point split used by current v1 responses.
    #[serde(default)]
    pub bps: Option<u16>,
    /// Additive route metadata retained verbatim for `/swap-instructions`.
    #[serde(flatten)]
    pub additional_fields: BTreeMap<String, Value>,
}

/// AMM metadata inside a Jupiter route leg.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JupiterSwapInfo {
    /// AMM pool or market address.
    pub amm_key: String,
    /// Human-readable route label supplied by Jupiter.
    pub label: String,
    /// Leg input mint.
    pub input_mint: String,
    /// Leg output mint.
    pub output_mint: String,
    /// Leg raw input amount.
    pub in_amount: String,
    /// Leg raw output amount.
    pub out_amount: String,
    /// Optional fee amount omitted by some current responses.
    #[serde(default)]
    pub fee_amount: Option<String>,
    /// Optional fee mint omitted by some current responses.
    #[serde(default)]
    pub fee_mint: Option<String>,
    /// Additive AMM metadata retained verbatim for `/swap-instructions`.
    #[serde(flatten)]
    pub additional_fields: BTreeMap<String, Value>,
}

/// Account metadata returned for one unsigned Jupiter instruction.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JupiterAccountMeta {
    /// Account public key.
    pub pubkey: String,
    /// Whether the message requires this account to sign.
    pub is_signer: bool,
    /// Whether the instruction requests write access.
    pub is_writable: bool,
}

/// Base64 instruction payload returned by Jupiter.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JupiterInstruction {
    /// Top-level program invoked by the instruction.
    pub program_id: String,
    /// Ordered account metas. Duplicate positions are retained for program ABI compatibility.
    pub accounts: Vec<JupiterAccountMeta>,
    /// Base64-encoded instruction data.
    pub data: String,
}

/// Unsigned response from Jupiter's `/swap-instructions` endpoint.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JupiterSwapInstructions {
    /// Optional token-ledger instruction. Strict policy rejects it.
    #[serde(default)]
    pub token_ledger_instruction: Option<JupiterInstruction>,
    /// Compute budget instructions.
    #[serde(default)]
    pub compute_budget_instructions: Vec<JupiterInstruction>,
    /// Account setup instructions.
    #[serde(default)]
    pub setup_instructions: Vec<JupiterInstruction>,
    /// The Jupiter aggregator instruction.
    pub swap_instruction: JupiterInstruction,
    /// Optional wrapped-SOL cleanup instruction.
    #[serde(default)]
    pub cleanup_instruction: Option<JupiterInstruction>,
    /// Additional instructions, including possible third-party tips. Strict policy rejects them.
    #[serde(default)]
    pub other_instructions: Vec<JupiterInstruction>,
    /// Address lookup tables that must be read from Surfpool.
    #[serde(default)]
    pub address_lookup_table_addresses: Vec<String>,
    /// Priority fee selected by Jupiter. Strict requests force this to zero.
    #[serde(default)]
    pub prioritization_fee_lamports: u64,
    /// Upstream simulation error, if Jupiter performed a simulation.
    #[serde(default)]
    pub simulation_error: Option<Value>,
}

/// The only Jupiter HTTP operations permitted by this integration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JupiterUnsignedEndpoint {
    /// Retrieve an unsigned exact-input quote.
    Quote,
    /// Retrieve unsigned instructions corresponding to a validated quote.
    SwapInstructions,
    /// Retrieve Jupiter's program-ID-to-label mapping for review tooling.
    ProgramIdToLabel,
}

impl JupiterUnsignedEndpoint {
    /// Return the exact Swap v1 endpoint path.
    #[must_use]
    pub const fn path(self) -> &'static str {
        match self {
            Self::Quote => "/quote",
            Self::SwapInstructions => "/swap-instructions",
            Self::ProgramIdToLabel => "/program-id-to-label",
        }
    }
}

impl TryFrom<&str> for JupiterUnsignedEndpoint {
    type Error = CookerError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "/quote" | "/swap/v1/quote" => Ok(Self::Quote),
            "/swap-instructions" | "/swap/v1/swap-instructions" => Ok(Self::SwapInstructions),
            "/program-id-to-label" | "/swap/v1/program-id-to-label" => Ok(Self::ProgramIdToLabel),
            _ => Err(CookerError::Policy(
                "Jupiter operation is not an approved unsigned planning endpoint".to_owned(),
            )),
        }
    }
}

/// Strict policy binding one exact-input intent to one reviewed Jupiter route family.
#[derive(Clone, Debug)]
pub struct JupiterPolicy {
    allowed_mints: BTreeSet<Pubkey>,
    forced_route_label: String,
    allowed_amm_accounts: BTreeSet<Pubkey>,
    allowed_accounts: BTreeSet<Pubkey>,
    allowed_lookup_tables: BTreeSet<Pubkey>,
    input_mint: Pubkey,
    output_mint: Pubkey,
    input_amount: u64,
    max_slippage_bps: u16,
    max_price_impact_bps: u16,
    signer: Pubkey,
    jupiter_program: Pubkey,
    allowed_route_programs: BTreeMap<Pubkey, String>,
    allowed_top_level_programs: BTreeSet<Pubkey>,
    allowed_writable_accounts: BTreeSet<Pubkey>,
}

impl JupiterPolicy {
    /// Create a reviewed route-manifest policy that is bound to an exact intent during prepare.
    /// AMM accounts are writable by definition; all other accounts remain read-only unless added
    /// through [`Self::with_writable_accounts`].
    ///
    /// # Errors
    ///
    /// Returns [`CookerError`] when required allowlists are empty, the route label is invalid, or
    /// the price-impact limit exceeds 10000 basis points.
    pub fn new(
        allowed_mints: BTreeSet<Pubkey>,
        forced_route_label: String,
        allowed_amm_accounts: BTreeSet<Pubkey>,
        allowed_programs: BTreeSet<Pubkey>,
        allowed_accounts: BTreeSet<Pubkey>,
        allowed_lookup_tables: BTreeSet<Pubkey>,
        max_price_impact_bps: u16,
    ) -> Result<Self, CookerError> {
        if allowed_mints.len() < 2 {
            return Err(CookerError::InvalidConfig(
                "Jupiter policy requires at least two allowed mints".to_owned(),
            ));
        }
        if forced_route_label.trim().is_empty() || forced_route_label.len() > 64 {
            return Err(CookerError::InvalidConfig(
                "Jupiter forced route label is empty or too long".to_owned(),
            ));
        }
        if allowed_amm_accounts.is_empty() || allowed_programs.is_empty() {
            return Err(CookerError::InvalidConfig(
                "Jupiter policy requires reviewed AMM and program allowlists".to_owned(),
            ));
        }
        if max_price_impact_bps > 10_000 {
            return Err(CookerError::InvalidConfig(
                "Jupiter price-impact limit cannot exceed 10000 bps".to_owned(),
            ));
        }
        let jupiter_program = parse_pubkey(JUPITER_V6_PROGRAM, "Jupiter program")?;
        let allowed_route_programs = allowed_programs
            .into_iter()
            .map(|program| (program, forced_route_label.clone()))
            .collect();
        let allowed_writable_accounts = allowed_amm_accounts.clone();
        Ok(Self {
            allowed_mints,
            forced_route_label,
            allowed_amm_accounts,
            allowed_accounts,
            allowed_lookup_tables,
            input_mint: Pubkey::default(),
            output_mint: Pubkey::default(),
            input_amount: 0,
            max_slippage_bps: 0,
            max_price_impact_bps,
            signer: Pubkey::default(),
            jupiter_program,
            allowed_route_programs,
            allowed_top_level_programs: canonical_top_level_programs(jupiter_program)?,
            allowed_writable_accounts,
        })
    }

    /// Create a strict, direct Raydium CLMM policy for the known Surfpool route family.
    ///
    /// The signer and its input/output associated token accounts are writable by default. Pool,
    /// vault, and tick-array addresses must be added from reviewed fixture metadata.
    ///
    /// # Errors
    ///
    /// Returns [`CookerError`] for a zero amount, identical mints, or limits outside basis-point
    /// bounds.
    pub fn strict_raydium_clmm(
        input_mint: Pubkey,
        output_mint: Pubkey,
        input_amount: u64,
        max_slippage_bps: u16,
        max_price_impact_bps: u16,
        signer: Pubkey,
    ) -> Result<Self, CookerError> {
        if input_amount == 0 {
            return Err(CookerError::Policy(
                "Jupiter input amount must be positive".to_owned(),
            ));
        }
        if input_mint == output_mint {
            return Err(CookerError::Policy(
                "Jupiter input and output mints must differ".to_owned(),
            ));
        }
        if max_slippage_bps >= 10_000 || max_price_impact_bps > 10_000 {
            return Err(CookerError::Policy(
                "Jupiter slippage and price-impact limits cannot exceed 10000 bps".to_owned(),
            ));
        }

        let jupiter_program = parse_pubkey(JUPITER_V6_PROGRAM, "Jupiter program")?;
        let raydium_program = parse_pubkey(RAYDIUM_CLMM_PROGRAM, "Raydium CLMM program")?;
        let allowed_top_level_programs = canonical_top_level_programs(jupiter_program)?;
        let allowed_writable_accounts = [
            signer,
            get_associated_token_address(&signer, &input_mint),
            get_associated_token_address(&signer, &output_mint),
        ]
        .into_iter()
        .collect();

        Ok(Self {
            allowed_mints: BTreeSet::from([input_mint, output_mint]),
            forced_route_label: "Raydium CLMM".to_owned(),
            allowed_amm_accounts: BTreeSet::new(),
            allowed_accounts: BTreeSet::new(),
            allowed_lookup_tables: BTreeSet::new(),
            input_mint,
            output_mint,
            input_amount,
            max_slippage_bps,
            max_price_impact_bps,
            signer,
            jupiter_program,
            allowed_route_programs: BTreeMap::from([(raydium_program, "Raydium CLMM".to_owned())]),
            allowed_top_level_programs,
            allowed_writable_accounts,
        })
    }

    /// Add reviewed writable accounts to this policy.
    #[must_use]
    pub fn with_writable_accounts(mut self, accounts: impl IntoIterator<Item = Pubkey>) -> Self {
        let accounts: Vec<_> = accounts.into_iter().collect();
        self.allowed_writable_accounts
            .extend(accounts.iter().copied());
        self.allowed_accounts.extend(accounts);
        self
    }

    /// Add reviewed read-only or writable account references to this policy.
    #[must_use]
    pub fn with_allowed_accounts(mut self, accounts: impl IntoIterator<Item = Pubkey>) -> Self {
        self.allowed_accounts.extend(accounts);
        self
    }

    /// Expected local signer.
    #[must_use]
    pub const fn signer(&self) -> Pubkey {
        self.signer
    }

    /// Exact raw input amount.
    #[must_use]
    pub const fn input_amount(&self) -> u64 {
        self.input_amount
    }

    fn bind_intent(
        &self,
        input_mint: Pubkey,
        output_mint: Pubkey,
        input_amount: u64,
        max_slippage_bps: u16,
        signer: Pubkey,
    ) -> Result<Self, CookerError> {
        if !self.allowed_mints.contains(&input_mint) || !self.allowed_mints.contains(&output_mint) {
            return Err(CookerError::Policy(
                "Jupiter action contains a mint outside the reviewed policy".to_owned(),
            ));
        }
        if input_mint == output_mint || input_amount == 0 || max_slippage_bps >= 10_000 {
            return Err(CookerError::Policy(
                "Jupiter action has invalid mints, amount, or slippage".to_owned(),
            ));
        }
        let mut bound = self.clone();
        bound.input_mint = input_mint;
        bound.output_mint = output_mint;
        bound.input_amount = input_amount;
        bound.max_slippage_bps = max_slippage_bps;
        bound.signer = signer;
        bound.allowed_writable_accounts.extend([
            signer,
            get_associated_token_address(&signer, &input_mint),
            get_associated_token_address(&signer, &output_mint),
        ]);
        bound.allowed_accounts.extend([
            signer,
            input_mint,
            output_mint,
            get_associated_token_address(&signer, &input_mint),
            get_associated_token_address(&signer, &output_mint),
        ]);
        Ok(bound)
    }

    /// Validate quote identity, amounts, thresholds, route, slippage, and price impact.
    ///
    /// # Errors
    ///
    /// Returns [`CookerError`] when any quote field is malformed or weakens the policy.
    pub fn validate_quote(&self, quote: &JupiterQuote) -> Result<(), CookerError> {
        let input_mint = parse_canonical_pubkey(&quote.input_mint, "quote input mint")?;
        let output_mint = parse_canonical_pubkey(&quote.output_mint, "quote output mint")?;
        let input_amount = parse_decimal_u64(&quote.in_amount, "quote input amount")?;
        let output_amount = parse_positive_decimal_u64(&quote.out_amount, "quote output amount")?;
        let threshold =
            parse_positive_decimal_u64(&quote.other_amount_threshold, "quote output threshold")?;

        if input_mint != self.input_mint
            || output_mint != self.output_mint
            || input_amount != self.input_amount
        {
            return Err(CookerError::Policy(
                "Jupiter quote does not match the exact mint and input-amount intent".to_owned(),
            ));
        }
        if quote.swap_mode != "ExactIn" {
            return Err(CookerError::Policy(
                "only Jupiter ExactIn quotes are accepted".to_owned(),
            ));
        }
        if quote.instruction_version != "V2" {
            return Err(CookerError::Policy(format!(
                "Jupiter quote instruction version {} is not V2",
                quote.instruction_version
            )));
        }
        if quote.slippage_bps > self.max_slippage_bps {
            return Err(CookerError::Policy(format!(
                "Jupiter quote slippage {} exceeds policy limit {}",
                quote.slippage_bps, self.max_slippage_bps
            )));
        }
        if !decimal_percent_le_bps(&quote.price_impact_pct, self.max_price_impact_bps)? {
            return Err(CookerError::Policy(format!(
                "Jupiter price impact {}% exceeds policy limit {} bps",
                quote.price_impact_pct, self.max_price_impact_bps
            )));
        }
        validate_zero_platform_fee(quote.platform_fee.as_ref())?;
        validate_output_threshold(output_amount, threshold, quote.slippage_bps)?;
        if !quote.time_taken.is_finite() || quote.time_taken < 0.0 {
            return Err(CookerError::Codec(
                "Jupiter quote timeTaken is invalid".to_owned(),
            ));
        }
        self.validate_route_plan(quote)
    }

    /// Convert and validate every unsigned instruction while preserving Jupiter's ABI order.
    ///
    /// # Errors
    ///
    /// Returns [`CookerError`] for unapproved programs, signers, writables, duplicate privilege
    /// escalation, malformed data, tips, token-ledger flows, or mismatched route accounts.
    pub fn validate_swap_instructions(
        &self,
        quote: &JupiterQuote,
        response: &JupiterSwapInstructions,
    ) -> Result<(Vec<Instruction>, Vec<Pubkey>), CookerError> {
        self.validate_quote(quote)?;
        if response.token_ledger_instruction.is_some() {
            return Err(CookerError::Policy(
                "Jupiter token-ledger instructions are not approved".to_owned(),
            ));
        }
        if !response.other_instructions.is_empty() {
            return Err(CookerError::Policy(
                "Jupiter otherInstructions, including tip flows, are not approved".to_owned(),
            ));
        }
        if response.prioritization_fee_lamports != 0 {
            return Err(CookerError::Policy(
                "Jupiter response added an unapproved priority fee".to_owned(),
            ));
        }
        if response.simulation_error.is_some() {
            return Err(CookerError::Policy(
                "Jupiter reported an upstream simulation error".to_owned(),
            ));
        }

        let instruction_count = response
            .compute_budget_instructions
            .len()
            .saturating_add(response.setup_instructions.len())
            .saturating_add(1)
            .saturating_add(usize::from(response.cleanup_instruction.is_some()));
        if instruction_count > MAX_INSTRUCTIONS {
            return Err(CookerError::Policy(format!(
                "Jupiter returned {instruction_count} instructions; limit is {MAX_INSTRUCTIONS}"
            )));
        }

        let compute_program = parse_pubkey(COMPUTE_BUDGET_PROGRAM, "compute budget program")?;
        let system_program = parse_pubkey(SYSTEM_PROGRAM, "system program")?;
        let token_program = parse_pubkey(TOKEN_PROGRAM, "classic token program")?;
        let associated_token_program =
            parse_pubkey(ASSOCIATED_TOKEN_PROGRAM, "associated token program")?;
        let setup_programs =
            BTreeSet::from([system_program, token_program, associated_token_program]);
        let cleanup_programs = BTreeSet::from([token_program]);
        let mut converted = Vec::with_capacity(instruction_count);

        for instruction in &response.compute_budget_instructions {
            let (instruction, _) = self.convert_instruction(instruction)?;
            if instruction.program_id != compute_program || !instruction.accounts.is_empty() {
                return Err(CookerError::Policy(
                    "compute budget section contained a non-compute instruction".to_owned(),
                ));
            }
            converted.push(instruction);
        }
        for instruction in &response.setup_instructions {
            let (instruction, _) = self.convert_instruction(instruction)?;
            if !setup_programs.contains(&instruction.program_id) {
                return Err(CookerError::Policy(
                    "setup section invoked an unapproved program".to_owned(),
                ));
            }
            converted.push(instruction);
        }

        let (swap_instruction, swap_privileges) =
            self.convert_instruction(&response.swap_instruction)?;
        if swap_instruction.program_id != self.jupiter_program {
            return Err(CookerError::Policy(
                "swapInstruction does not invoke the approved Jupiter program".to_owned(),
            ));
        }
        if !privilege_has_signer(&swap_privileges, self.signer) {
            return Err(CookerError::Policy(
                "Jupiter swap instruction omitted the expected local signer".to_owned(),
            ));
        }
        self.validate_swap_accounts(quote, &swap_privileges)?;
        converted.push(swap_instruction);

        if let Some(instruction) = response.cleanup_instruction.as_ref() {
            let (instruction, _) = self.convert_instruction(instruction)?;
            if !cleanup_programs.contains(&instruction.program_id) {
                return Err(CookerError::Policy(
                    "cleanup section invoked an unapproved program".to_owned(),
                ));
            }
            converted.push(instruction);
        }

        let lookup_tables = parse_lookup_table_addresses(response)?;
        Ok((converted, lookup_tables))
    }

    /// Verify that Surfpool-resolved lookup tables exactly match the instruction response.
    ///
    /// # Errors
    ///
    /// Returns [`CookerError`] for missing, extra, empty, or duplicate lookup tables.
    /// Repeated addresses inside or across real tables are legal; Solana's pinned v0 compiler
    /// deterministically selects the first matching table entry.
    pub fn validate_lookup_tables(
        &self,
        response: &JupiterSwapInstructions,
        resolved: &[AddressLookupTableAccount],
    ) -> Result<(), CookerError> {
        let expected: BTreeSet<_> = parse_lookup_table_addresses(response)?
            .into_iter()
            .collect();
        let actual: BTreeSet<_> = resolved.iter().map(|table| table.key).collect();
        if actual.len() != resolved.len() || actual != expected {
            return Err(CookerError::Policy(
                "resolved lookup-table set differs from Jupiter's validated response".to_owned(),
            ));
        }
        if !self.allowed_lookup_tables.is_empty() && !actual.is_subset(&self.allowed_lookup_tables)
        {
            return Err(CookerError::Policy(
                "Jupiter requested a lookup table outside the reviewed manifest".to_owned(),
            ));
        }
        for table in resolved {
            if table.addresses.is_empty() {
                return Err(CookerError::Codec(format!(
                    "lookup table {} contained no addresses",
                    table.key
                )));
            }
        }
        Ok(())
    }

    fn validate_route_plan(&self, quote: &JupiterQuote) -> Result<(), CookerError> {
        if quote.route_plan.is_empty() || quote.route_plan.len() > MAX_ROUTE_STEPS {
            return Err(CookerError::Policy(format!(
                "Jupiter route must contain 1..={MAX_ROUTE_STEPS} legs"
            )));
        }
        let mut total_bps = 0_u32;
        let mut legs = Vec::with_capacity(quote.route_plan.len());
        for step in &quote.route_plan {
            let leg_bps = route_step_bps(step)?;
            total_bps = total_bps
                .checked_add(u32::from(leg_bps))
                .ok_or_else(|| CookerError::Codec("Jupiter route split overflowed".to_owned()))?;
            let pool = parse_canonical_pubkey(&step.swap_info.amm_key, "route AMM key")?;
            if !self.allowed_amm_accounts.is_empty() && !self.allowed_amm_accounts.contains(&pool) {
                return Err(CookerError::Policy(format!(
                    "Jupiter selected unreviewed AMM account {pool}"
                )));
            }
            let input = parse_canonical_pubkey(&step.swap_info.input_mint, "route input mint")?;
            let output = parse_canonical_pubkey(&step.swap_info.output_mint, "route output mint")?;
            parse_positive_decimal_u64(&step.swap_info.in_amount, "route input amount")?;
            parse_positive_decimal_u64(&step.swap_info.out_amount, "route output amount")?;
            if input == output {
                return Err(CookerError::Policy(
                    "Jupiter route contains a self-swap leg".to_owned(),
                ));
            }
            if let Some(fee_amount) = step.swap_info.fee_amount.as_deref() {
                parse_decimal_u64(fee_amount, "route fee amount")?;
            }
            if let Some(fee_mint) = step.swap_info.fee_mint.as_deref() {
                parse_canonical_pubkey(fee_mint, "route fee mint")?;
            }
            if step.swap_info.label != self.forced_route_label
                || !self
                    .allowed_route_programs
                    .values()
                    .any(|label| label == &step.swap_info.label)
            {
                return Err(CookerError::Policy(format!(
                    "Jupiter route label {} is not allowlisted",
                    step.swap_info.label
                )));
            }
            legs.push((pool, input, output));
        }
        if total_bps != 10_000 {
            return Err(CookerError::Policy(format!(
                "Jupiter route split totals {total_bps} bps instead of 10000"
            )));
        }
        validate_route_connectivity(self.input_mint, self.output_mint, &legs)
    }

    fn convert_instruction(
        &self,
        value: &JupiterInstruction,
    ) -> Result<(Instruction, BTreeMap<Pubkey, Privilege>), CookerError> {
        let program_id = parse_canonical_pubkey(&value.program_id, "instruction program")?;
        if !self.allowed_top_level_programs.contains(&program_id) {
            return Err(CookerError::Policy(format!(
                "Jupiter instruction invokes unapproved program {program_id}"
            )));
        }
        if value.accounts.len() > MAX_ACCOUNTS_PER_INSTRUCTION {
            return Err(CookerError::Policy(format!(
                "Jupiter instruction has {} accounts; limit is {MAX_ACCOUNTS_PER_INSTRUCTION}",
                value.accounts.len()
            )));
        }
        let data = BASE64_STANDARD.decode(&value.data).map_err(|error| {
            CookerError::Codec(format!("invalid Jupiter instruction base64: {error}"))
        })?;
        if data.is_empty() || data.len() > MAX_INSTRUCTION_DATA_BYTES {
            return Err(CookerError::Policy(format!(
                "Jupiter instruction data must contain 1..={MAX_INSTRUCTION_DATA_BYTES} bytes"
            )));
        }

        let mut accounts = Vec::with_capacity(value.accounts.len());
        let mut first_privileges = BTreeMap::<Pubkey, Privilege>::new();
        for account in &value.accounts {
            let pubkey = parse_canonical_pubkey(&account.pubkey, "instruction account")?;
            if !self.allowed_accounts.is_empty()
                && !self.allowed_accounts.contains(&pubkey)
                && !self.allowed_top_level_programs.contains(&pubkey)
                && !self.allowed_route_programs.contains_key(&pubkey)
                && !self.allowed_amm_accounts.contains(&pubkey)
            {
                return Err(CookerError::Policy(format!(
                    "Jupiter instruction referenced unreviewed account {pubkey}"
                )));
            }
            let privilege = Privilege {
                signer: account.is_signer,
                writable: account.is_writable,
            };
            if account.is_signer && pubkey != self.signer {
                return Err(CookerError::Policy(format!(
                    "Jupiter requested unapproved signer {pubkey}"
                )));
            }
            if account.is_writable && !self.allowed_writable_accounts.contains(&pubkey) {
                return Err(CookerError::Policy(format!(
                    "Jupiter requested unreviewed writable account {pubkey}"
                )));
            }
            if pubkey == program_id && (account.is_signer || account.is_writable) {
                return Err(CookerError::Policy(
                    "instruction program account requested elevated privileges".to_owned(),
                ));
            }
            first_privileges
                .entry(pubkey)
                .and_modify(|effective| {
                    effective.signer |= privilege.signer;
                    effective.writable |= privilege.writable;
                })
                .or_insert(privilege);
            accounts.push(if account.is_writable {
                AccountMeta::new(pubkey, account.is_signer)
            } else {
                AccountMeta::new_readonly(pubkey, account.is_signer)
            });
        }

        Ok((
            Instruction {
                program_id,
                accounts,
                data,
            },
            first_privileges,
        ))
    }

    fn validate_swap_accounts(
        &self,
        quote: &JupiterQuote,
        privileges: &BTreeMap<Pubkey, Privilege>,
    ) -> Result<(), CookerError> {
        for required in [self.input_mint, self.output_mint] {
            if !privileges.contains_key(&required) {
                return Err(CookerError::Policy(format!(
                    "Jupiter swap instruction omitted mint {required}"
                )));
            }
        }
        for step in &quote.route_plan {
            let pool = parse_canonical_pubkey(&step.swap_info.amm_key, "route AMM key")?;
            if !privileges.contains_key(&pool) {
                return Err(CookerError::Policy(format!(
                    "Jupiter swap instruction omitted quoted AMM account {pool}"
                )));
            }
            let programs: Vec<_> = self
                .allowed_route_programs
                .iter()
                .filter_map(|(program, label)| {
                    (label.as_str() == step.swap_info.label).then_some(*program)
                })
                .collect();
            let present = programs.iter().find_map(|program| {
                privileges
                    .get(program)
                    .map(|privilege| (*program, privilege))
            });
            let Some((program, privilege)) = present else {
                return Err(CookerError::Policy(
                    "Jupiter swap instruction omitted every allowlisted route program".to_owned(),
                ));
            };
            if privilege.signer || privilege.writable {
                return Err(CookerError::Policy(format!(
                    "route program {program} requested elevated privileges"
                )));
            }
        }
        Ok(())
    }
}

/// HTTP client limited to Jupiter's unsigned planning endpoints.
#[derive(Clone)]
pub struct JupiterApiClient {
    client: Client,
    base_url: &'static str,
    api_key: Option<String>,
}

impl fmt::Debug for JupiterApiClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("JupiterApiClient")
            .field("base_url", &self.base_url)
            .field("has_api_key", &self.api_key.is_some())
            .finish_non_exhaustive()
    }
}

impl Drop for JupiterApiClient {
    fn drop(&mut self) {
        if let Some(api_key) = self.api_key.as_mut() {
            api_key.zeroize();
        }
    }
}

impl JupiterApiClient {
    /// Create the authenticated official production planning client.
    ///
    /// # Errors
    ///
    /// Returns [`CookerError`] when the API key is empty or the HTTP client cannot be built.
    pub fn production(api_key: String) -> Result<Self, CookerError> {
        if api_key.trim().is_empty() {
            return Err(CookerError::InvalidConfig(
                "Jupiter API key cannot be empty".to_owned(),
            ));
        }
        Self::new(Some(api_key))
    }

    /// Create an official Jupiter client. A key selects `api.jup.ag`; no key selects the official
    /// lite planning endpoint.
    ///
    /// # Errors
    ///
    /// Returns [`CookerError`] when the hardened HTTP client cannot be constructed.
    pub fn new(api_key: Option<String>) -> Result<Self, CookerError> {
        let client = Client::builder()
            .no_proxy()
            .redirect(RedirectPolicy::none())
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|error| {
                CookerError::InvalidConfig(format!("cannot build Jupiter HTTP client: {error}"))
            })?;
        let base_url = if api_key.is_some() {
            JUPITER_API_BASE
        } else {
            JUPITER_LITE_API_BASE
        };
        Ok(Self {
            client,
            base_url,
            api_key,
        })
    }

    /// Request and validate an unsigned quote for an exact policy intent.
    ///
    /// # Errors
    ///
    /// Returns [`CookerError`] for transport, status, decoding, or policy failure.
    pub async fn quote(&self, policy: &JupiterPolicy) -> Result<JupiterQuote, CookerError> {
        let route_label = policy
            .allowed_route_programs
            .values()
            .next()
            .ok_or_else(|| CookerError::InvalidConfig("route allowlist is empty".to_owned()))?;
        let request = JupiterQuoteRequest {
            input_mint: policy.input_mint.to_string(),
            output_mint: policy.output_mint.to_string(),
            amount: policy.input_amount,
            slippage_bps: policy.max_slippage_bps,
            swap_mode: "ExactIn",
            dexes: route_label,
            only_direct_routes: true,
            restrict_intermediate_tokens: true,
            as_legacy_transaction: false,
            max_accounts: 64,
            instruction_version: "V2",
        };
        let quote: JupiterQuote = self.get(JupiterUnsignedEndpoint::Quote, &request).await?;
        policy.validate_quote(&quote)?;
        Ok(quote)
    }

    /// Request unsigned instructions for an already validated quote.
    ///
    /// # Errors
    ///
    /// Returns [`CookerError`] for transport, status, decoding, or policy failure.
    pub async fn swap_instructions(
        &self,
        policy: &JupiterPolicy,
        quote: &JupiterQuote,
    ) -> Result<JupiterSwapInstructions, CookerError> {
        policy.validate_quote(quote)?;
        let request = JupiterSwapInstructionsRequest {
            user_public_key: policy.signer.to_string(),
            quote_response: quote,
            wrap_and_unwrap_sol: true,
            use_shared_accounts: false,
            prioritization_fee_lamports: 0,
            as_legacy_transaction: false,
            dynamic_compute_unit_limit: false,
            skip_user_accounts_rpc_calls: true,
            dynamic_slippage: false,
        };
        let response: JupiterSwapInstructions = self
            .post(JupiterUnsignedEndpoint::SwapInstructions, &request)
            .await?;
        policy.validate_swap_instructions(quote, &response)?;
        Ok(response)
    }

    async fn get<T: DeserializeOwned, Q: Serialize + ?Sized>(
        &self,
        endpoint: JupiterUnsignedEndpoint,
        query: &Q,
    ) -> Result<T, CookerError> {
        let request = self
            .with_auth(
                self.client
                    .get(format!("{}{path}", self.base_url, path = endpoint.path())),
            )
            .query(query);
        self.decode_response(request.send().await).await
    }

    async fn post<T: DeserializeOwned, B: Serialize + ?Sized>(
        &self,
        endpoint: JupiterUnsignedEndpoint,
        body: &B,
    ) -> Result<T, CookerError> {
        let request = self
            .with_auth(
                self.client
                    .post(format!("{}{path}", self.base_url, path = endpoint.path())),
            )
            .json(body);
        self.decode_response(request.send().await).await
    }

    fn with_auth(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match self.api_key.as_ref() {
            Some(key) => request.header("x-api-key", key),
            None => request,
        }
    }

    async fn decode_response<T: DeserializeOwned>(
        &self,
        response: Result<reqwest::Response, reqwest::Error>,
    ) -> Result<T, CookerError> {
        let response = response
            .map_err(|error| CookerError::Chain(format!("Jupiter request failed: {error}")))?;
        let status = response.status();
        let bytes = response
            .bytes()
            .await
            .map_err(|error| CookerError::Chain(format!("Jupiter response failed: {error}")))?;
        if bytes.len() > MAX_HTTP_RESPONSE_BYTES {
            return Err(CookerError::Codec(
                "Jupiter response exceeded the configured size limit".to_owned(),
            ));
        }
        if !status.is_success() {
            let detail: String = String::from_utf8_lossy(&bytes).chars().take(512).collect();
            return Err(CookerError::Chain(format!(
                "Jupiter returned HTTP {}: {}",
                status.as_u16(),
                detail
            )));
        }
        serde_json::from_slice(&bytes)
            .map_err(|error| CookerError::Codec(format!("invalid Jupiter response: {error}")))
    }
}

/// Coordinator for unsigned Jupiter planning and local transaction assembly.
#[derive(Clone, Debug)]
pub struct JupiterAdapter {
    gateway: Arc<SolanaGateway>,
    signer: Arc<LocalKeypair>,
    client: Arc<JupiterApiClient>,
    policy: JupiterPolicy,
}

impl JupiterAdapter {
    /// Create an adapter from a verified Surfpool gateway, local signer, planner, and policy.
    #[must_use]
    pub const fn new(
        gateway: Arc<SolanaGateway>,
        signer: Arc<LocalKeypair>,
        client: Arc<JupiterApiClient>,
        policy: JupiterPolicy,
    ) -> Self {
        Self {
            gateway,
            signer,
            client,
            policy,
        }
    }

    /// Assemble a previously captured and reviewed unsigned Jupiter plan against current
    /// Surfpool state.
    ///
    /// This path is intended for reproducible acceptance evidence. It performs the same intent,
    /// account-privilege, route, balance, lookup-table, and local-signing checks as the live
    /// planner path; only the unsigned HTTP fetch is replaced by caller-supplied JSON.
    ///
    /// # Errors
    ///
    /// Returns [`CookerError`] when the action, captured plan, Surfpool state, or signer violates
    /// policy, or when local transaction assembly fails.
    pub async fn prepare_from_reviewed_plan(
        &self,
        context: &AdapterContext,
        action: &PlannedAction,
        quote: &JupiterQuote,
        response: &JupiterSwapInstructions,
    ) -> Result<PreparedAction, CookerError> {
        let (bound, input_mint, output_mint) = self.bind_action(context, action).await?;
        self.assemble_plan(action, &bound, input_mint, output_mint, quote, response)
            .await
    }

    async fn bind_action(
        &self,
        context: &AdapterContext,
        action: &PlannedAction,
    ) -> Result<(JupiterPolicy, Pubkey, Pubkey), CookerError> {
        validate_adapter_context(context, &self.gateway, &self.signer)?;
        let ActionPayload::JupiterSwap {
            input_mint,
            output_mint,
            amount,
            max_slippage_bps,
        } = &action.payload
        else {
            return Err(CookerError::Policy(
                "Jupiter adapter received another action type".to_owned(),
            ));
        };
        let input_mint = parse_canonical_pubkey(input_mint, "action input mint")?;
        let output_mint = parse_canonical_pubkey(output_mint, "action output mint")?;
        let bound = self.policy.bind_intent(
            input_mint,
            output_mint,
            *amount,
            *max_slippage_bps,
            self.signer.pubkey(),
        )?;
        validate_pre_swap_balances(
            &self.gateway,
            &self.signer,
            action,
            input_mint,
            output_mint,
            *amount,
        )
        .await?;
        Ok((bound, input_mint, output_mint))
    }

    async fn assemble_plan(
        &self,
        action: &PlannedAction,
        bound: &JupiterPolicy,
        input_mint: Pubkey,
        output_mint: Pubkey,
        quote: &JupiterQuote,
        response: &JupiterSwapInstructions,
    ) -> Result<PreparedAction, CookerError> {
        bound.validate_quote(quote)?;
        let lookup_tables = resolve_lookup_tables(&self.gateway, response).await?;
        bound.validate_lookup_tables(response, &lookup_tables)?;
        let (instructions, _) = bound.validate_swap_instructions(quote, response)?;
        let latest = self.gateway.latest_blockhash().await?;
        let signed =
            build_signed_v0_transaction(&instructions, &lookup_tables, &self.signer, latest)?;
        let expectations = build_swap_expectations(
            quote,
            bound,
            action,
            self.signer.pubkey(),
            input_mint,
            output_mint,
        )?;
        Ok(PreparedAction {
            action_id: action.id.clone(),
            signature: signed.signature.to_string(),
            transaction: signed.bytes,
            recent_blockhash: signed.recent_blockhash,
            last_valid_block_height: signed.last_valid_block_height,
            expectations,
        })
    }
}

#[async_trait::async_trait]
impl ActionAdapter for JupiterAdapter {
    fn supports(&self, payload: &ActionPayload) -> bool {
        matches!(payload, ActionPayload::JupiterSwap { .. })
    }

    async fn prepare(
        &self,
        context: &AdapterContext,
        action: &PlannedAction,
    ) -> Result<PreparedAction, CookerError> {
        let (bound, input_mint, output_mint) = self.bind_action(context, action).await?;
        let quote = self.client.quote(&bound).await?;
        let response = self.client.swap_instructions(&bound, &quote).await?;
        self.assemble_plan(action, &bound, input_mint, output_mint, &quote, &response)
            .await
    }

    async fn observe(
        &self,
        context: &AdapterContext,
        action: &PlannedAction,
        prepared: &PreparedAction,
    ) -> Result<ChainReceipt, CookerError> {
        validate_adapter_context(context, &self.gateway, &self.signer)?;
        validate_prepared_swap(action, prepared, &self.signer)?;
        let ActionPayload::JupiterSwap {
            input_mint,
            output_mint,
            amount,
            max_slippage_bps: _,
        } = &action.payload
        else {
            return Err(CookerError::Policy(
                "Jupiter adapter received another action type".to_owned(),
            ));
        };
        let input_mint = parse_canonical_pubkey(input_mint, "action input mint")?;
        let output_mint = parse_canonical_pubkey(output_mint, "action output mint")?;
        let mut receipt = observe_until_terminal(&self.gateway, context, prepared).await?;
        if !matches!(
            receipt.status,
            ConfirmationStatus::Confirmed | ConfirmationStatus::Finalized
        ) {
            return Ok(receipt);
        }
        let signature = Signature::from_str(&prepared.signature)
            .map_err(|error| CookerError::Codec(format!("invalid Jupiter signature: {error}")))?;
        let record = self.gateway.transaction(&signature).await?.ok_or_else(|| {
            CookerError::Chain("confirmed Jupiter swap had no transaction metadata".to_owned())
        })?;
        let input_expectation = find_swap_expectation(prepared, JUPITER_INPUT_KIND)?;
        let output_expectation = find_swap_expectation(prepared, JUPITER_OUTPUT_KIND)?;
        let route_expectation = find_swap_expectation(prepared, JUPITER_ROUTE_KIND)?;
        let minimum_output = expectation_u64(output_expectation, "minimum_delta")?;

        let (input_delta, input_met) = observed_input_delta(
            &record,
            self.signer.pubkey(),
            input_mint,
            *amount,
            action.max_account_creation_lamports,
        )?;
        let (output_delta, output_met) =
            observed_output_delta(&record, self.signer.pubkey(), output_mint, minimum_output)?;
        let fee_met = record.fee <= action.max_fee_lamports;
        let route_met = route_logs_match(&record.logs, route_expectation)?;
        receipt.observations = vec![
            observed_swap_expectation(input_expectation, input_delta, input_met),
            observed_swap_expectation(output_expectation, output_delta, output_met),
            observed_swap_expectation(route_expectation, 0, route_met),
            StateExpectation {
                kind: "transaction_fee_ceiling".to_owned(),
                account: self.signer.pubkey().to_string(),
                expected_delta: None,
                attributes: BTreeMap::from([
                    ("actual_fee_lamports".to_owned(), record.fee.to_string()),
                    (
                        "max_fee_lamports".to_owned(),
                        action.max_fee_lamports.to_string(),
                    ),
                    ("met".to_owned(), fee_met.to_string()),
                ]),
            },
        ];
        receipt.postconditions_met = input_met && output_met && route_met && fee_met;
        if !receipt.postconditions_met {
            receipt.error = Some("Jupiter swap postconditions were not proven".to_owned());
        }
        Ok(receipt)
    }
}

fn validate_adapter_context(
    context: &AdapterContext,
    gateway: &SolanaGateway,
    signer: &LocalKeypair,
) -> Result<(), CookerError> {
    if context.rpc_url != *gateway.endpoint().as_url() {
        return Err(CookerError::InvalidConfig(
            "Jupiter adapter context differs from the verified Surfpool gateway".to_owned(),
        ));
    }
    let context_signer = parse_canonical_pubkey(&context.signer, "adapter signer")?;
    if context_signer != signer.pubkey() {
        return Err(CookerError::InvalidConfig(
            "Jupiter adapter context differs from the loaded signer".to_owned(),
        ));
    }
    Ok(())
}

async fn validate_pre_swap_balances(
    gateway: &SolanaGateway,
    signer: &LocalKeypair,
    action: &PlannedAction,
    input_mint: Pubkey,
    _output_mint: Pubkey,
    amount: u64,
) -> Result<(), CookerError> {
    let native_required = action
        .max_fee_lamports
        .checked_add(action.max_account_creation_lamports)
        .and_then(|required| {
            let asset_input = if input_mint.to_string() == WRAPPED_SOL_MINT {
                amount
            } else {
                0
            };
            asset_input.checked_add(required)
        })
        .ok_or_else(|| CookerError::Policy("Jupiter native spend overflowed".to_owned()))?;
    let native_balance = gateway.balance(&signer.pubkey()).await?;
    if native_balance < native_required {
        return Err(CookerError::Policy(format!(
            "native balance {native_balance} cannot cover Jupiter spend ceiling {native_required}"
        )));
    }
    if input_mint.to_string() != WRAPPED_SOL_MINT {
        let source = get_associated_token_address(&signer.pubkey(), &input_mint);
        let source_amount =
            read_classic_token_amount(gateway, &source, &input_mint, &signer.pubkey())
                .await?
                .ok_or_else(|| {
                    CookerError::NotFound(format!("Jupiter source token account {source}"))
                })?;
        if source_amount < amount {
            return Err(CookerError::Policy(format!(
                "Jupiter source token balance {source_amount} cannot cover {amount}"
            )));
        }
    }
    Ok(())
}

async fn read_classic_token_amount(
    gateway: &SolanaGateway,
    address: &Pubkey,
    mint: &Pubkey,
    owner: &Pubkey,
) -> Result<Option<u64>, CookerError> {
    let Some(account) = gateway.account(address).await? else {
        return Ok(None);
    };
    if account.owner != spl_token_interface::id() {
        return Err(CookerError::Policy(format!(
            "Jupiter token account {address} is not classic SPL Token"
        )));
    }
    let token = TokenAccount::unpack(&account.data)
        .map_err(|error| CookerError::Codec(format!("invalid token account {address}: {error}")))?;
    if token.mint != *mint || token.owner != *owner || token.is_frozen() {
        return Err(CookerError::Policy(format!(
            "Jupiter token account {address} has unexpected mint, owner, or state"
        )));
    }
    Ok(Some(token.amount))
}

async fn resolve_lookup_tables(
    gateway: &SolanaGateway,
    response: &JupiterSwapInstructions,
) -> Result<Vec<AddressLookupTableAccount>, CookerError> {
    let addresses = parse_lookup_table_addresses(response)?;
    let mut resolved = Vec::with_capacity(addresses.len());
    for address in addresses {
        let account = gateway
            .account(&address)
            .await?
            .ok_or_else(|| CookerError::NotFound(format!("Jupiter lookup table {address}")))?;
        if account.owner != lookup_table_program::id() || account.executable {
            return Err(CookerError::Policy(format!(
                "Jupiter lookup table {address} has an invalid owner or executable flag"
            )));
        }
        let table = AddressLookupTable::deserialize(&account.data).map_err(|error| {
            CookerError::Codec(format!("invalid Jupiter lookup table {address}: {error}"))
        })?;
        if table.meta.deactivation_slot != u64::MAX {
            return Err(CookerError::Policy(format!(
                "Jupiter lookup table {address} is deactivating or deactivated"
            )));
        }
        let active_len = if account.context_slot > table.meta.last_extended_slot {
            table.addresses.len()
        } else {
            usize::from(table.meta.last_extended_slot_start_index)
        };
        let active = table.addresses.get(..active_len).ok_or_else(|| {
            CookerError::Codec(format!("lookup table {address} active length is invalid"))
        })?;
        resolved.push(AddressLookupTableAccount {
            key: address,
            addresses: active.to_vec(),
        });
    }
    Ok(resolved)
}

fn build_swap_expectations(
    quote: &JupiterQuote,
    policy: &JupiterPolicy,
    action: &PlannedAction,
    signer: Pubkey,
    input_mint: Pubkey,
    output_mint: Pubkey,
) -> Result<Vec<StateExpectation>, CookerError> {
    let minimum_output =
        parse_positive_decimal_u64(&quote.other_amount_threshold, "quote output threshold")?;
    let quoted_output = parse_positive_decimal_u64(&quote.out_amount, "quote output amount")?;
    let input_account = if input_mint.to_string() == WRAPPED_SOL_MINT {
        signer
    } else {
        get_associated_token_address(&signer, &input_mint)
    };
    let output_account = if output_mint.to_string() == WRAPPED_SOL_MINT {
        signer
    } else {
        get_associated_token_address(&signer, &output_mint)
    };
    let route_programs = policy
        .allowed_route_programs
        .keys()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let amm_accounts = quote
        .route_plan
        .iter()
        .map(|step| step.swap_info.amm_key.as_str())
        .collect::<Vec<_>>()
        .join(",");
    let route_account = quote
        .route_plan
        .first()
        .map(|step| step.swap_info.amm_key.clone())
        .ok_or_else(|| CookerError::Codec("Jupiter quote has no route".to_owned()))?;
    Ok(vec![
        StateExpectation {
            kind: JUPITER_INPUT_KIND.to_owned(),
            account: input_account.to_string(),
            expected_delta: Some(-i128::from(action.payload.input_amount())),
            attributes: BTreeMap::from([
                ("mint".to_owned(), input_mint.to_string()),
                (
                    "raw_amount".to_owned(),
                    action.payload.input_amount().to_string(),
                ),
                (
                    "source_kind".to_owned(),
                    if input_account == signer {
                        "native_wrapped_sol"
                    } else {
                        "token_account"
                    }
                    .to_owned(),
                ),
            ]),
        },
        StateExpectation {
            kind: JUPITER_OUTPUT_KIND.to_owned(),
            account: output_account.to_string(),
            expected_delta: None,
            attributes: BTreeMap::from([
                ("mint".to_owned(), output_mint.to_string()),
                ("minimum_delta".to_owned(), minimum_output.to_string()),
                ("quoted_output".to_owned(), quoted_output.to_string()),
                (
                    "destination_kind".to_owned(),
                    if output_account == signer {
                        "native_unwrapped_sol"
                    } else {
                        "token_account"
                    }
                    .to_owned(),
                ),
            ]),
        },
        StateExpectation {
            kind: JUPITER_ROUTE_KIND.to_owned(),
            account: route_account,
            expected_delta: None,
            attributes: BTreeMap::from([
                ("amm_accounts".to_owned(), amm_accounts),
                (
                    "jupiter_program".to_owned(),
                    policy.jupiter_program.to_string(),
                ),
                ("route_label".to_owned(), policy.forced_route_label.clone()),
                ("route_programs".to_owned(), route_programs),
                (
                    "price_impact_pct".to_owned(),
                    quote.price_impact_pct.clone(),
                ),
            ]),
        },
    ])
}

fn validate_prepared_swap(
    action: &PlannedAction,
    prepared: &PreparedAction,
    signer: &LocalKeypair,
) -> Result<(), CookerError> {
    if prepared.action_id != action.id {
        return Err(CookerError::Codec(
            "prepared Jupiter transaction belongs to another action".to_owned(),
        ));
    }
    let expected = Signature::from_str(&prepared.signature)
        .map_err(|error| CookerError::Codec(format!("invalid Jupiter signature: {error}")))?;
    let transaction: VersionedTransaction = bincode::deserialize(&prepared.transaction)
        .map_err(|error| CookerError::Codec(format!("invalid Jupiter transaction: {error}")))?;
    if transaction.signatures.first() != Some(&expected)
        || transaction.message.static_account_keys().first() != Some(&signer.pubkey())
        || transaction.message.recent_blockhash().to_string() != prepared.recent_blockhash
        || !expected.verify(signer.pubkey().as_ref(), &transaction.message.serialize())
    {
        return Err(CookerError::Codec(
            "prepared Jupiter transaction metadata or signature is inconsistent".to_owned(),
        ));
    }
    Ok(())
}

fn find_swap_expectation<'a>(
    prepared: &'a PreparedAction,
    kind: &str,
) -> Result<&'a StateExpectation, CookerError> {
    prepared
        .expectations
        .iter()
        .find(|expectation| expectation.kind == kind)
        .ok_or_else(|| CookerError::Codec(format!("prepared Jupiter swap omitted {kind}")))
}

fn expectation_u64(expectation: &StateExpectation, key: &str) -> Result<u64, CookerError> {
    expectation
        .attributes
        .get(key)
        .ok_or_else(|| CookerError::Codec(format!("Jupiter expectation omitted {key}")))?
        .parse()
        .map_err(|error| CookerError::Codec(format!("invalid Jupiter {key}: {error}")))
}

fn observed_input_delta(
    record: &crate::TransactionRecord,
    signer: Pubkey,
    mint: Pubkey,
    amount: u64,
    max_account_creation_lamports: u64,
) -> Result<(i128, bool), CookerError> {
    if mint.to_string() == WRAPPED_SOL_MINT {
        let (before, after) = native_record_balances(record, signer)?;
        let debit = before.checked_sub(after).ok_or_else(|| {
            CookerError::Policy("wrapped-SOL input did not debit native balance".to_owned())
        })?;
        let minimum = amount
            .checked_add(record.fee)
            .ok_or_else(|| CookerError::Codec("Jupiter debit overflowed".to_owned()))?;
        let maximum = minimum
            .checked_add(max_account_creation_lamports)
            .ok_or_else(|| CookerError::Codec("Jupiter debit ceiling overflowed".to_owned()))?;
        return Ok((-i128::from(debit), debit >= minimum && debit <= maximum));
    }
    let account = get_associated_token_address(&signer, &mint);
    let before = transaction_token_amount(record, true, account, mint, signer)?;
    let after = transaction_token_amount(record, false, account, mint, signer)?;
    let delta = i128::from(after) - i128::from(before);
    Ok((delta, delta == -i128::from(amount)))
}

fn observed_output_delta(
    record: &crate::TransactionRecord,
    signer: Pubkey,
    mint: Pubkey,
    minimum_output: u64,
) -> Result<(i128, bool), CookerError> {
    if mint.to_string() == WRAPPED_SOL_MINT {
        let (before, after) = native_record_balances(record, signer)?;
        let adjusted = i128::from(after) - i128::from(before) + i128::from(record.fee);
        return Ok((adjusted, adjusted >= i128::from(minimum_output)));
    }
    let account = get_associated_token_address(&signer, &mint);
    let before =
        optional_transaction_token_amount(record, true, account, mint, signer)?.unwrap_or(0);
    let after = transaction_token_amount(record, false, account, mint, signer)?;
    let delta = i128::from(after) - i128::from(before);
    Ok((delta, delta >= i128::from(minimum_output)))
}

fn native_record_balances(
    record: &crate::TransactionRecord,
    account: Pubkey,
) -> Result<(u64, u64), CookerError> {
    let index = record
        .account_keys
        .iter()
        .position(|candidate| *candidate == account)
        .ok_or_else(|| CookerError::Codec("transaction omitted signer balance".to_owned()))?;
    let before = *record
        .pre_balances
        .get(index)
        .ok_or_else(|| CookerError::Codec("transaction omitted pre-balance".to_owned()))?;
    let after = *record
        .post_balances
        .get(index)
        .ok_or_else(|| CookerError::Codec("transaction omitted post-balance".to_owned()))?;
    Ok((before, after))
}

fn transaction_token_amount(
    record: &crate::TransactionRecord,
    before: bool,
    account: Pubkey,
    mint: Pubkey,
    owner: Pubkey,
) -> Result<u64, CookerError> {
    optional_transaction_token_amount(record, before, account, mint, owner)?.ok_or_else(|| {
        CookerError::Codec(format!(
            "transaction metadata omitted token balance for {account}"
        ))
    })
}

fn optional_transaction_token_amount(
    record: &crate::TransactionRecord,
    before: bool,
    account: Pubkey,
    mint: Pubkey,
    owner: Pubkey,
) -> Result<Option<u64>, CookerError> {
    let balance = if before {
        record.pre_token_balance(&account, &mint)
    } else {
        record.post_token_balance(&account, &mint)
    };
    let Some(balance) = balance else {
        return Ok(None);
    };
    if balance.owner != Some(owner) || balance.program_id != Some(spl_token_interface::id()) {
        return Err(CookerError::Policy(format!(
            "transaction token evidence for {account} has unexpected owner or program"
        )));
    }
    Ok(Some(balance.amount))
}

fn route_logs_match(logs: &[String], expectation: &StateExpectation) -> Result<bool, CookerError> {
    let jupiter = expectation
        .attributes
        .get("jupiter_program")
        .ok_or_else(|| {
            CookerError::Codec("route expectation omitted Jupiter program".to_owned())
        })?;
    let programs = expectation
        .attributes
        .get("route_programs")
        .ok_or_else(|| CookerError::Codec("route expectation omitted route programs".to_owned()))?;
    let jupiter_seen = logs.iter().any(|log| log.contains(jupiter));
    let route_seen = programs
        .split(',')
        .filter(|program| !program.is_empty())
        .any(|program| logs.iter().any(|log| log.contains(program)));
    Ok(jupiter_seen && route_seen)
}

fn observed_swap_expectation(
    expectation: &StateExpectation,
    actual_delta: i128,
    met: bool,
) -> StateExpectation {
    let mut observed = expectation.clone();
    observed
        .attributes
        .insert("actual_delta".to_owned(), actual_delta.to_string());
    observed
        .attributes
        .insert("met".to_owned(), met.to_string());
    observed
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct JupiterQuoteRequest<'a> {
    input_mint: String,
    output_mint: String,
    amount: u64,
    slippage_bps: u16,
    swap_mode: &'static str,
    dexes: &'a str,
    only_direct_routes: bool,
    restrict_intermediate_tokens: bool,
    as_legacy_transaction: bool,
    max_accounts: u8,
    instruction_version: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(
    clippy::struct_excessive_bools,
    reason = "explicit false values lock down Jupiter's independent unsafe request switches"
)]
struct JupiterSwapInstructionsRequest<'a> {
    user_public_key: String,
    quote_response: &'a JupiterQuote,
    wrap_and_unwrap_sol: bool,
    use_shared_accounts: bool,
    prioritization_fee_lamports: u64,
    as_legacy_transaction: bool,
    dynamic_compute_unit_limit: bool,
    skip_user_accounts_rpc_calls: bool,
    dynamic_slippage: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Privilege {
    signer: bool,
    writable: bool,
}

fn parse_lookup_table_addresses(
    response: &JupiterSwapInstructions,
) -> Result<Vec<Pubkey>, CookerError> {
    if response.address_lookup_table_addresses.len() > MAX_LOOKUP_TABLES {
        return Err(CookerError::Policy(format!(
            "Jupiter returned too many lookup tables; limit is {MAX_LOOKUP_TABLES}"
        )));
    }
    let mut seen = BTreeSet::new();
    let mut parsed = Vec::with_capacity(response.address_lookup_table_addresses.len());
    for address in &response.address_lookup_table_addresses {
        let address = parse_canonical_pubkey(address, "lookup table")?;
        if !seen.insert(address) {
            return Err(CookerError::Policy(format!(
                "Jupiter repeated lookup table {address}"
            )));
        }
        parsed.push(address);
    }
    Ok(parsed)
}

fn validate_zero_platform_fee(fee: Option<&JupiterPlatformFee>) -> Result<(), CookerError> {
    if let Some(fee) = fee {
        let amount = parse_decimal_u64(&fee.amount, "platform fee amount")?;
        if amount != 0 || fee.fee_bps != 0 {
            return Err(CookerError::Policy(
                "Jupiter platform fees are not approved".to_owned(),
            ));
        }
    }
    Ok(())
}

fn validate_output_threshold(
    output_amount: u64,
    threshold: u64,
    slippage_bps: u16,
) -> Result<(), CookerError> {
    let numerator = u128::from(output_amount) * u128::from(10_000 - slippage_bps);
    let expected = numerator.div_ceil(10_000);
    if u128::from(threshold) != expected || threshold > output_amount {
        return Err(CookerError::Policy(format!(
            "Jupiter output threshold {threshold} is inconsistent with quote output {output_amount} and slippage {slippage_bps}"
        )));
    }
    Ok(())
}

fn route_step_bps(step: &JupiterRouteStep) -> Result<u16, CookerError> {
    match (step.percent, step.bps) {
        (Some(percent), Some(bps)) if u32::from(percent) * 100 == u32::from(bps) => Ok(bps),
        (Some(_), Some(_)) => Err(CookerError::Policy(
            "Jupiter route percent and bps disagree".to_owned(),
        )),
        (Some(percent), None) if percent <= 100 => Ok(percent * 100),
        (None, Some(bps)) if bps <= 10_000 => Ok(bps),
        _ => Err(CookerError::Policy(
            "Jupiter route leg has an invalid split".to_owned(),
        )),
    }
}

fn validate_route_connectivity(
    input_mint: Pubkey,
    output_mint: Pubkey,
    legs: &[(Pubkey, Pubkey, Pubkey)],
) -> Result<(), CookerError> {
    let mut reachable = BTreeSet::from([input_mint]);
    let mut used = vec![false; legs.len()];
    loop {
        let mut changed = false;
        for (index, (_, input, output)) in legs.iter().enumerate() {
            if !used[index] && reachable.contains(input) {
                used[index] = true;
                reachable.insert(*output);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    if !reachable.contains(&output_mint) || used.iter().any(|used| !used) {
        return Err(CookerError::Policy(
            "Jupiter route is disconnected from the requested mint pair".to_owned(),
        ));
    }
    Ok(())
}

fn privilege_has_signer(privileges: &BTreeMap<Pubkey, Privilege>, signer: Pubkey) -> bool {
    privileges.get(&signer).is_some_and(|value| value.signer)
}

fn decimal_percent_le_bps(value: &str, max_bps: u16) -> Result<bool, CookerError> {
    if value.is_empty() || value.len() > 128 || !value.is_ascii() {
        return Err(CookerError::Codec(
            "Jupiter price impact is not a bounded ASCII decimal".to_owned(),
        ));
    }
    let mut parts = value.split('.');
    let integer = parts.next().unwrap_or_default();
    let fraction = parts.next().unwrap_or("");
    if parts.next().is_some()
        || integer.is_empty()
        || (integer.len() > 1 && integer.starts_with('0'))
        || (value.contains('.') && fraction.is_empty())
        || !integer.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(CookerError::Codec(
            "Jupiter price impact is not a plain unsigned decimal".to_owned(),
        ));
    }
    let integer = integer.trim_start_matches('0');
    let integer = if integer.is_empty() { "0" } else { integer };
    let max_integer = (max_bps / 100).to_string();
    match integer.len().cmp(&max_integer.len()) {
        std::cmp::Ordering::Less => return Ok(true),
        std::cmp::Ordering::Greater => return Ok(false),
        std::cmp::Ordering::Equal => match integer.cmp(max_integer.as_str()) {
            std::cmp::Ordering::Less => return Ok(true),
            std::cmp::Ordering::Greater => return Ok(false),
            std::cmp::Ordering::Equal => {}
        },
    }
    let maximum_fraction = format!("{:02}", max_bps % 100);
    let compared_len = fraction.len().max(maximum_fraction.len());
    for index in 0..compared_len {
        let actual = fraction.as_bytes().get(index).copied().unwrap_or(b'0');
        let maximum = maximum_fraction
            .as_bytes()
            .get(index)
            .copied()
            .unwrap_or(b'0');
        match actual.cmp(&maximum) {
            std::cmp::Ordering::Less => return Ok(true),
            std::cmp::Ordering::Greater => return Ok(false),
            std::cmp::Ordering::Equal => {}
        }
    }
    Ok(true)
}

fn parse_positive_decimal_u64(value: &str, label: &str) -> Result<u64, CookerError> {
    let value = parse_decimal_u64(value, label)?;
    if value == 0 {
        return Err(CookerError::Policy(format!("{label} must be positive")));
    }
    Ok(value)
}

fn parse_decimal_u64(value: &str, label: &str) -> Result<u64, CookerError> {
    if value.is_empty()
        || !value.bytes().all(|byte| byte.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return Err(CookerError::Codec(format!(
            "{label} is not a canonical unsigned integer"
        )));
    }
    value
        .parse()
        .map_err(|error| CookerError::Codec(format!("invalid {label}: {error}")))
}

fn parse_canonical_pubkey(value: &str, label: &str) -> Result<Pubkey, CookerError> {
    let parsed = parse_pubkey(value, label)?;
    if parsed.to_string() != value {
        return Err(CookerError::Codec(format!(
            "{label} is not canonically encoded"
        )));
    }
    Ok(parsed)
}

fn canonical_top_level_programs(jupiter_program: Pubkey) -> Result<BTreeSet<Pubkey>, CookerError> {
    Ok([
        jupiter_program,
        parse_pubkey(COMPUTE_BUDGET_PROGRAM, "compute budget program")?,
        parse_pubkey(SYSTEM_PROGRAM, "system program")?,
        parse_pubkey(TOKEN_PROGRAM, "classic token program")?,
        parse_pubkey(ASSOCIATED_TOKEN_PROGRAM, "associated token program")?,
    ]
    .into_iter()
    .collect())
}

#[cfg(test)]
#[allow(
    clippy::too_many_lines,
    reason = "security policy mutations are intentionally explicit"
)]
mod tests {
    use solana_hash::Hash;
    use solana_keypair::Keypair;
    use solana_message::VersionedMessage;
    use solana_transaction::versioned::VersionedTransaction;

    use super::*;
    use crate::LatestBlockhash;

    const QUOTE_FIXTURE: &str =
        include_str!("../../../fixtures/surfpool/jupiter/raydium-clmm-sol-usdc.quote.json");
    const INSTRUCTIONS_FIXTURE: &str =
        include_str!("../../../fixtures/surfpool/jupiter/raydium-clmm-sol-usdc.instructions.json");
    const CURRENT_QUOTE_FIXTURE: &str = include_str!(
        "../../../fixtures/surfpool/jupiter/raydium-clmm-sol-usdc-cyb-433411234.quote.json"
    );
    const CURRENT_INSTRUCTIONS_FIXTURE: &str = include_str!(
        "../../../fixtures/surfpool/jupiter/raydium-clmm-sol-usdc-cyb-433411234.instructions.json"
    );
    const FIXTURE_SIGNER: &str = "5nNamRwUKNUCK272DkjaMWtghCBLwFSaGiLU5oCdwYJr";

    #[test]
    fn recorded_unsigned_responses_parse_and_validate() -> Result<(), Box<dyn std::error::Error>> {
        let quote = fixture_quote()?;
        let response = fixture_instructions()?;
        let policy = fixture_policy(&quote, &response)?;

        policy.validate_quote(&quote)?;
        let (instructions, tables) = policy.validate_swap_instructions(&quote, &response)?;

        assert_eq!(instructions.len(), 7);
        assert_eq!(tables.len(), 1);
        assert_eq!(quote.route_plan[0].bps, Some(10_000));
        assert_eq!(quote.route_plan[0].percent, None);
        assert_eq!(quote.instruction_version, "V2");
        assert!(quote.additional_fields.contains_key("swapUsdValue"));
        assert!(
            quote.route_plan[0]
                .swap_info
                .additional_fields
                .contains_key("updateContextSlot")
        );

        let request = JupiterSwapInstructionsRequest {
            user_public_key: FIXTURE_SIGNER.to_owned(),
            quote_response: &quote,
            wrap_and_unwrap_sol: true,
            use_shared_accounts: false,
            prioritization_fee_lamports: 0,
            as_legacy_transaction: false,
            dynamic_compute_unit_limit: false,
            skip_user_accounts_rpc_calls: true,
            dynamic_slippage: false,
        };
        let posted = serde_json::to_value(request)?;
        let original: Value = serde_json::from_str(QUOTE_FIXTURE)?;
        assert_eq!(
            posted.pointer("/quoteResponse/instructionVersion"),
            original.pointer("/instructionVersion")
        );
        assert_eq!(
            posted.pointer("/quoteResponse/swapUsdValue"),
            original.pointer("/swapUsdValue")
        );
        assert_eq!(
            posted.pointer("/quoteResponse/mostReliableAmmsQuoteReport"),
            original.pointer("/mostReliableAmmsQuoteReport")
        );
        assert_eq!(
            posted.pointer("/quoteResponse/routePlan/0/swapInfo/updateContextSlot"),
            original.pointer("/routePlan/0/swapInfo/updateContextSlot")
        );
        Ok(())
    }

    #[test]
    fn quote_binding_rejects_amount_mint_slippage_and_price_impact()
    -> Result<(), Box<dyn std::error::Error>> {
        let quote = fixture_quote()?;
        let response = fixture_instructions()?;
        let policy = fixture_policy(&quote, &response)?;

        let mut wrong_amount = quote.clone();
        wrong_amount.in_amount = "100000001".to_owned();
        assert!(policy.validate_quote(&wrong_amount).is_err());

        let mut wrong_mint = quote.clone();
        wrong_mint.output_mint = wrong_mint.input_mint.clone();
        assert!(policy.validate_quote(&wrong_mint).is_err());

        let mut excessive_slippage = quote.clone();
        excessive_slippage.slippage_bps = 51;
        assert!(policy.validate_quote(&excessive_slippage).is_err());

        let mut legacy_instruction_schema = quote.clone();
        legacy_instruction_schema.instruction_version = "V1".to_owned();
        assert!(policy.validate_quote(&legacy_instruction_schema).is_err());

        let strict_impact = JupiterPolicy::strict_raydium_clmm(
            Pubkey::from_str(&quote.input_mint)?,
            Pubkey::from_str(&quote.output_mint)?,
            100_000_000,
            50,
            0,
            Pubkey::from_str(FIXTURE_SIGNER)?,
        )?;
        assert!(strict_impact.validate_quote(&quote).is_err());
        Ok(())
    }

    #[test]
    fn current_unsigned_response_preserves_exact_writable_manifest()
    -> Result<(), Box<dyn std::error::Error>> {
        let quote: JupiterQuote = serde_json::from_str(CURRENT_QUOTE_FIXTURE)?;
        let response: JupiterSwapInstructions = serde_json::from_str(CURRENT_INSTRUCTIONS_FIXTURE)?;
        let signer = response
            .swap_instruction
            .accounts
            .iter()
            .find(|account| account.is_signer)
            .ok_or("current fixture omitted signer")?;
        let policy = fixture_policy_for_signer(
            &quote,
            &response,
            parse_pubkey(&signer.pubkey, "current fixture signer")?,
        )?;

        policy.validate_quote(&quote)?;
        policy.validate_swap_instructions(&quote, &response)?;

        let mut escalated = response.clone();
        let readonly = escalated
            .swap_instruction
            .accounts
            .iter_mut()
            .find(|account| account.pubkey == "D8cy77BBepLMngZx6ZukaTff5hCt1HrWyKk3Hnd9oitf")
            .ok_or("current fixture omitted reviewed read-only account")?;
        readonly.is_writable = true;
        assert!(
            policy
                .validate_swap_instructions(&quote, &escalated)
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn route_and_instruction_privilege_attacks_are_rejected()
    -> Result<(), Box<dyn std::error::Error>> {
        let quote = fixture_quote()?;
        let response = fixture_instructions()?;
        let policy = fixture_policy(&quote, &response)?;

        let mut unapproved_route = quote.clone();
        unapproved_route.route_plan[0].swap_info.label = "Unreviewed DEX".to_owned();
        assert!(policy.validate_quote(&unapproved_route).is_err());

        let mut extra_signer = response.clone();
        extra_signer.swap_instruction.accounts[1].is_signer = true;
        assert!(
            policy
                .validate_swap_instructions(&quote, &extra_signer)
                .is_err()
        );

        let mut duplicate_escalation = response.clone();
        let token_program = spl_token_interface::id();
        let second_token_program_position = duplicate_escalation
            .swap_instruction
            .accounts
            .iter()
            .enumerate()
            .filter(|(_, meta)| meta.pubkey == token_program.to_string())
            .nth(1)
            .map(|(index, _)| index)
            .ok_or("fixture omitted duplicate token program")?;
        duplicate_escalation.swap_instruction.accounts[second_token_program_position].is_writable =
            true;
        assert!(
            policy
                .validate_swap_instructions(&quote, &duplicate_escalation)
                .is_err()
        );

        let mut tipped = response.clone();
        tipped
            .other_instructions
            .push(tipped.swap_instruction.clone());
        assert!(policy.validate_swap_instructions(&quote, &tipped).is_err());
        Ok(())
    }

    #[test]
    fn signed_or_submitted_jupiter_operations_have_no_endpoint_variant() {
        assert!(JupiterUnsignedEndpoint::try_from("/swap/v1/swap").is_err());
        assert!(JupiterUnsignedEndpoint::try_from("/execute").is_err());
        assert!(JupiterUnsignedEndpoint::try_from("/submit").is_err());
        assert!(matches!(
            JupiterUnsignedEndpoint::try_from("/swap/v1/swap-instructions"),
            Ok(JupiterUnsignedEndpoint::SwapInstructions)
        ));
    }

    #[test]
    fn validated_fixture_can_only_be_signed_locally_as_v0() -> Result<(), Box<dyn std::error::Error>>
    {
        let quote = fixture_quote()?;
        let mut response = fixture_instructions()?;
        let signer = LocalKeypair::from_keypair_for_test(Keypair::new());
        replace_signer(&mut response, FIXTURE_SIGNER, signer.pubkey());
        let policy = fixture_policy_for_signer(&quote, &response, signer.pubkey())?;

        let (instructions, lookup_addresses) =
            policy.validate_swap_instructions(&quote, &response)?;
        let all_accounts = response_account_keys(&response)?;
        let mut addresses: Vec<_> = all_accounts
            .into_iter()
            .filter(|account| *account != signer.pubkey())
            .collect();
        let repeated_program = spl_token_interface::id();
        if !addresses.contains(&repeated_program) {
            return Err("current fixture omitted the SPL Token program".into());
        }
        addresses.push(repeated_program);
        let lookup_table = AddressLookupTableAccount {
            key: *lookup_addresses
                .first()
                .ok_or("missing fixture lookup table")?,
            addresses,
        };
        policy.validate_lookup_tables(&response, std::slice::from_ref(&lookup_table))?;
        let wire_transaction = build_signed_v0_transaction(
            &instructions,
            &[lookup_table],
            &signer,
            LatestBlockhash {
                context_slot: 1,
                blockhash: Hash::new_from_array([17; 32]),
                last_valid_block_height: 22,
            },
        )?;
        let transaction: VersionedTransaction = bincode::deserialize(&wire_transaction.bytes)?;

        assert!(matches!(transaction.message, VersionedMessage::V0(_)));
        assert!(
            wire_transaction
                .signature
                .verify(signer.pubkey().as_ref(), &transaction.message.serialize())
        );
        Ok(())
    }

    fn fixture_quote() -> Result<JupiterQuote, serde_json::Error> {
        serde_json::from_str(QUOTE_FIXTURE)
    }

    fn fixture_instructions() -> Result<JupiterSwapInstructions, serde_json::Error> {
        serde_json::from_str(INSTRUCTIONS_FIXTURE)
    }

    fn fixture_policy(
        quote: &JupiterQuote,
        response: &JupiterSwapInstructions,
    ) -> Result<JupiterPolicy, CookerError> {
        fixture_policy_for_signer(
            quote,
            response,
            parse_pubkey(FIXTURE_SIGNER, "fixture signer")?,
        )
    }

    fn fixture_policy_for_signer(
        quote: &JupiterQuote,
        response: &JupiterSwapInstructions,
        signer: Pubkey,
    ) -> Result<JupiterPolicy, CookerError> {
        let writable = response_writable_accounts(response)?;
        let all_accounts = response_account_keys(response)?;
        JupiterPolicy::strict_raydium_clmm(
            parse_pubkey(&quote.input_mint, "fixture input mint")?,
            parse_pubkey(&quote.output_mint, "fixture output mint")?,
            100_000_000,
            50,
            100,
            signer,
        )
        .map(|policy| {
            policy
                .with_writable_accounts(writable)
                .with_allowed_accounts(all_accounts)
        })
    }

    fn response_writable_accounts(
        response: &JupiterSwapInstructions,
    ) -> Result<BTreeSet<Pubkey>, CookerError> {
        let mut result = BTreeSet::new();
        visit_response_instructions(response, |instruction| {
            for meta in &instruction.accounts {
                if meta.is_writable {
                    result.insert(parse_pubkey(&meta.pubkey, "fixture writable")?);
                }
            }
            Ok(())
        })?;
        Ok(result)
    }

    fn response_account_keys(
        response: &JupiterSwapInstructions,
    ) -> Result<BTreeSet<Pubkey>, CookerError> {
        let mut result = BTreeSet::new();
        visit_response_instructions(response, |instruction| {
            result.insert(parse_pubkey(&instruction.program_id, "fixture program")?);
            for meta in &instruction.accounts {
                result.insert(parse_pubkey(&meta.pubkey, "fixture account")?);
            }
            Ok(())
        })?;
        Ok(result)
    }

    fn visit_response_instructions(
        response: &JupiterSwapInstructions,
        mut visitor: impl FnMut(&JupiterInstruction) -> Result<(), CookerError>,
    ) -> Result<(), CookerError> {
        for instruction in &response.compute_budget_instructions {
            visitor(instruction)?;
        }
        for instruction in &response.setup_instructions {
            visitor(instruction)?;
        }
        visitor(&response.swap_instruction)?;
        if let Some(instruction) = response.cleanup_instruction.as_ref() {
            visitor(instruction)?;
        }
        Ok(())
    }

    fn replace_signer(response: &mut JupiterSwapInstructions, old: &str, new: Pubkey) {
        let replacement = new.to_string();
        for instruction in &mut response.compute_budget_instructions {
            replace_instruction_signer(instruction, old, &replacement);
        }
        for instruction in &mut response.setup_instructions {
            replace_instruction_signer(instruction, old, &replacement);
        }
        replace_instruction_signer(&mut response.swap_instruction, old, &replacement);
        if let Some(instruction) = response.cleanup_instruction.as_mut() {
            replace_instruction_signer(instruction, old, &replacement);
        }
    }

    fn replace_instruction_signer(instruction: &mut JupiterInstruction, old: &str, new: &str) {
        for meta in &mut instruction.accounts {
            if meta.pubkey == old {
                meta.pubkey = new.to_owned();
            }
        }
    }
}

//! Async JSON-RPC transport and verified Surfpool gateway.

use std::{
    fmt,
    str::FromStr,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use chrono::Utc;
use cooker_core::{
    ActionId, ChainGateway, ChainReceipt, ConfirmationStatus, CookerError, SimulationReceipt,
};
use reqwest::{Client, redirect::Policy as RedirectPolicy};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};
use solana_hash::Hash;
use solana_pubkey::Pubkey;
use solana_signature::Signature;
use solana_transaction::versioned::VersionedTransaction;

use crate::{RpcError, SurfpoolRpcUrl};

const MAX_RPC_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const GET_VERSION: &str = "getVersion";
const GET_SURFNET_INFO: &str = "surfnet_getSurfnetInfo";
const GET_LATEST_BLOCKHASH: &str = "getLatestBlockhash";
const GET_BLOCK_HEIGHT: &str = "getBlockHeight";
const GET_EPOCH_INFO: &str = "getEpochInfo";
const GET_STAKE_MINIMUM_DELEGATION: &str = "getStakeMinimumDelegation";
const GET_VOTE_ACCOUNTS: &str = "getVoteAccounts";
const GET_BALANCE: &str = "getBalance";
const GET_ACCOUNT_INFO: &str = "getAccountInfo";
const GET_MINIMUM_RENT: &str = "getMinimumBalanceForRentExemption";
const SIMULATE_TRANSACTION: &str = "simulateTransaction";
const SEND_TRANSACTION: &str = "sendTransaction";
const GET_SIGNATURE_STATUSES: &str = "getSignatureStatuses";
const GET_TRANSACTION: &str = "getTransaction";
const GET_LOCAL_SIGNATURES: &str = "surfnet_getLocalSignatures";
const TIME_TRAVEL: &str = "surfnet_timeTravel";

/// Surfpool version pinned by the local development harness.
pub const EXPECTED_SURFPOOL_VERSION: &str = "1.4.0";

/// Identity returned by the verified local Surfpool endpoint.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SurfpoolIdentity {
    /// Surfpool's own semantic version.
    pub surfnet_version: String,
    /// Embedded Solana runtime version when reported.
    pub solana_core: Option<String>,
    /// Active feature-set identifier when reported.
    pub feature_set: Option<u64>,
}

/// Latest blockhash plus the validity horizon returned by Surfpool.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LatestBlockhash {
    /// Context slot used for the response.
    pub context_slot: u64,
    /// Recent blockhash used to sign a transaction.
    pub blockhash: Hash,
    /// Final block height at which this blockhash remains valid.
    pub last_valid_block_height: u64,
}

/// Current Surfpool epoch and slot position.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EpochInfo {
    /// Current epoch.
    pub epoch: u64,
    /// Slot relative to the start of the epoch.
    pub slot_index: u64,
    /// Total slots in the current epoch.
    pub slots_in_epoch: u64,
    /// Absolute slot.
    pub absolute_slot: u64,
    /// Current block height.
    pub block_height: u64,
    /// Successful transaction count when Surfpool reports it.
    pub transaction_count: Option<u64>,
}

/// Active validator vote account returned by Surfpool's mainnet fork.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VoteAccount {
    /// Vote account address.
    pub vote_pubkey: Pubkey,
    /// Stake currently activated on the validator.
    pub activated_stake: u64,
}

/// One transaction retained in Surfpool's local-signature history.
#[derive(Clone, Debug, PartialEq)]
pub struct LocalSignatureRecord {
    /// Local transaction signature.
    pub signature: Signature,
    /// Execution error when the transaction failed.
    pub error: Option<Value>,
    /// Program logs retained by Surfpool.
    pub logs: Vec<String>,
}

/// Result of a local transaction simulation.
#[derive(Clone, Debug, PartialEq)]
pub struct RpcSimulation {
    /// Context slot used for simulation.
    pub context_slot: u64,
    /// Structured transaction error, if execution failed.
    pub error: Option<Value>,
    /// Program logs emitted by the local execution.
    pub logs: Vec<String>,
    /// Compute units consumed when reported.
    pub units_consumed: Option<u64>,
}

/// Signature state returned by `getSignatureStatuses`.
#[derive(Clone, Debug, PartialEq)]
pub struct SignatureStatus {
    /// Slot in which the local transaction was processed.
    pub slot: u64,
    /// Number of confirmations when known.
    pub confirmations: Option<u64>,
    /// Structured transaction error.
    pub error: Option<Value>,
    /// Surfpool's confirmation status string.
    pub confirmation_status: Option<String>,
}

/// Transaction metadata used for confirmation and postcondition evidence.
#[derive(Clone, Debug, PartialEq)]
pub struct TransactionRecord {
    /// Slot containing the local transaction.
    pub slot: u64,
    /// Local block timestamp when reported.
    pub block_time: Option<i64>,
    /// Transaction execution error.
    pub error: Option<Value>,
    /// Fee charged in lamports.
    pub fee: u64,
    /// Program logs retained by Surfpool.
    pub logs: Vec<String>,
    /// Native balances before execution.
    pub pre_balances: Vec<u64>,
    /// Native balances after execution.
    pub post_balances: Vec<u64>,
    /// Static and loaded account keys in transaction index order.
    pub account_keys: Vec<Pubkey>,
    /// Token balances captured before execution.
    pub pre_token_balances: Vec<TokenBalanceRecord>,
    /// Token balances captured after execution.
    pub post_token_balances: Vec<TokenBalanceRecord>,
}

impl TransactionRecord {
    /// Find pre-transaction token metadata for an indexed account and mint.
    #[must_use]
    pub fn pre_token_balance(
        &self,
        account: &Pubkey,
        mint: &Pubkey,
    ) -> Option<&TokenBalanceRecord> {
        self.token_balance(&self.pre_token_balances, account, mint)
    }

    /// Find post-transaction token metadata for an indexed account and mint.
    #[must_use]
    pub fn post_token_balance(
        &self,
        account: &Pubkey,
        mint: &Pubkey,
    ) -> Option<&TokenBalanceRecord> {
        self.token_balance(&self.post_token_balances, account, mint)
    }

    /// Find the pre-transaction raw token balance for an indexed account and mint.
    #[must_use]
    pub fn pre_token_amount(&self, account: &Pubkey, mint: &Pubkey) -> Option<u64> {
        self.pre_token_balance(account, mint)
            .map(|balance| balance.amount)
    }

    /// Find the post-transaction raw token balance for an indexed account and mint.
    #[must_use]
    pub fn post_token_amount(&self, account: &Pubkey, mint: &Pubkey) -> Option<u64> {
        self.post_token_balance(account, mint)
            .map(|balance| balance.amount)
    }

    fn token_balance<'a>(
        &self,
        balances: &'a [TokenBalanceRecord],
        account: &Pubkey,
        mint: &Pubkey,
    ) -> Option<&'a TokenBalanceRecord> {
        balances.iter().find(|balance| {
            balance.mint == *mint && self.account_keys.get(balance.account_index) == Some(account)
        })
    }
}

/// One raw token balance entry from confirmed transaction metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokenBalanceRecord {
    /// Index into [`TransactionRecord::account_keys`].
    pub account_index: usize,
    /// Token mint.
    pub mint: Pubkey,
    /// Wallet owner when Surfpool reports it.
    pub owner: Option<Pubkey>,
    /// Token program when Surfpool reports it.
    pub program_id: Option<Pubkey>,
    /// Raw token units without decimal conversion.
    pub amount: u64,
    /// Mint decimals reported with the balance.
    pub decimals: u8,
}

/// Raw account state read from the verified Surfpool endpoint.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountInfo {
    /// Context slot used for the read.
    pub context_slot: u64,
    /// Account lamports.
    pub lamports: u64,
    /// Owning program.
    pub owner: Pubkey,
    /// Whether this is executable program state.
    pub executable: bool,
    /// Rent epoch reported by Surfpool.
    pub rent_epoch: u64,
    /// Decoded account bytes.
    pub data: Vec<u8>,
}

#[derive(Clone)]
struct JsonRpcClient {
    endpoint: SurfpoolRpcUrl,
    client: Client,
    next_id: Arc<AtomicU64>,
}

impl fmt::Debug for JsonRpcClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("JsonRpcClient")
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

impl JsonRpcClient {
    fn new(endpoint: SurfpoolRpcUrl) -> Result<Self, RpcError> {
        let client = Client::builder()
            .no_proxy()
            .redirect(RedirectPolicy::none())
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|error| RpcError::invalid_input("connect", error.to_string()))?;
        Ok(Self {
            endpoint,
            client,
            next_id: Arc::new(AtomicU64::new(1)),
        })
    }

    async fn request<T: DeserializeOwned>(
        &self,
        method: &'static str,
        params: Value,
    ) -> Result<T, RpcError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let response = self
            .client
            .post(self.endpoint.as_url().clone())
            .json(&json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": method,
                "params": params,
            }))
            .send()
            .await
            .map_err(|error| RpcError::transport(method, &error))?;
        let status = response.status();
        let body = response
            .bytes()
            .await
            .map_err(|error| RpcError::transport(method, &error))?;
        if !status.is_success() {
            return Err(RpcError::http(
                method,
                status.as_u16(),
                String::from_utf8_lossy(&body),
            ));
        }
        if body.len() > MAX_RPC_RESPONSE_BYTES {
            return Err(RpcError::invalid_response(
                method,
                "response exceeded the configured size limit",
            ));
        }

        let envelope: Value = serde_json::from_slice(&body).map_err(|error| {
            RpcError::invalid_response(method, format!("invalid JSON: {error}"))
        })?;
        if envelope.get("id").and_then(Value::as_u64) != Some(id) {
            return Err(RpcError::invalid_response(
                method,
                "response id did not match request id",
            ));
        }
        if let Some(error) = envelope.get("error").filter(|value| !value.is_null()) {
            let code = error.get("code").and_then(Value::as_i64).unwrap_or(-32_603);
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("Surfpool returned an unspecified JSON-RPC error");
            return Err(RpcError::json_rpc(method, code, message));
        }
        let result = envelope.get("result").ok_or_else(|| {
            RpcError::invalid_response(method, "response did not contain result or error")
        })?;
        serde_json::from_value(result.clone()).map_err(|error| {
            RpcError::invalid_response(method, format!("result shape was invalid: {error}"))
        })
    }

    async fn version(&self) -> Result<SurfpoolIdentity, RpcError> {
        let result: VersionResponse = self.request(GET_VERSION, json!([])).await?;
        let surfnet_version = result
            .surfnet_version
            .ok_or_else(|| RpcError::identity(GET_VERSION, "getVersion omitted surfnet-version"))?;
        if surfnet_version != EXPECTED_SURFPOOL_VERSION {
            return Err(RpcError::identity(
                GET_VERSION,
                format!("expected Surfpool {EXPECTED_SURFPOOL_VERSION}, got {surfnet_version}"),
            ));
        }
        Ok(SurfpoolIdentity {
            surfnet_version,
            solana_core: result.solana_core,
            feature_set: result.feature_set,
        })
    }

    async fn verify_surfnet_info(&self) -> Result<(), RpcError> {
        let result: SurfnetInfoEnvelope = self.request(GET_SURFNET_INFO, json!([])).await?;
        if !result.value.is_object() {
            return Err(RpcError::identity(
                GET_SURFNET_INFO,
                "surfnet_getSurfnetInfo result was not an object",
            ));
        }
        Ok(())
    }

    async fn latest_blockhash(&self) -> Result<LatestBlockhash, RpcError> {
        let result: ContextResponse<LatestBlockhashValue> = self
            .request(GET_LATEST_BLOCKHASH, json!([{"commitment": "confirmed"}]))
            .await?;
        let blockhash = Hash::from_str(&result.value.blockhash).map_err(|error| {
            RpcError::invalid_response(GET_LATEST_BLOCKHASH, format!("invalid blockhash: {error}"))
        })?;
        Ok(LatestBlockhash {
            context_slot: result.context.slot,
            blockhash,
            last_valid_block_height: result.value.last_valid_block_height,
        })
    }

    async fn balance(&self, address: &Pubkey) -> Result<u64, RpcError> {
        let result: ContextResponse<u64> = self
            .request(
                GET_BALANCE,
                json!([address.to_string(), {"commitment": "confirmed"}]),
            )
            .await?;
        Ok(result.value)
    }

    async fn block_height(&self) -> Result<u64, RpcError> {
        self.request(GET_BLOCK_HEIGHT, json!([{"commitment": "confirmed"}]))
            .await
    }

    async fn account(&self, address: &Pubkey) -> Result<Option<AccountInfo>, RpcError> {
        let result: ContextResponse<Option<AccountValue>> = self
            .request(
                GET_ACCOUNT_INFO,
                json!([
                    address.to_string(),
                    {"commitment": "confirmed", "encoding": "base64"}
                ]),
            )
            .await?;
        result
            .value
            .as_ref()
            .map(|value| decode_account(result.context.slot, value))
            .transpose()
    }

    async fn minimum_balance_for_rent_exemption(&self, data_len: usize) -> Result<u64, RpcError> {
        self.request(
            GET_MINIMUM_RENT,
            json!([data_len, {"commitment": "confirmed"}]),
        )
        .await
    }

    async fn epoch_info(&self) -> Result<EpochInfo, RpcError> {
        let value: EpochInfoValue = self
            .request(GET_EPOCH_INFO, json!([{"commitment": "confirmed"}]))
            .await?;
        Ok(value.into())
    }

    async fn stake_minimum_delegation(&self) -> Result<u64, RpcError> {
        let result: ContextResponse<u64> = self
            .request(
                GET_STAKE_MINIMUM_DELEGATION,
                json!([{"commitment": "confirmed"}]),
            )
            .await?;
        Ok(result.value)
    }

    async fn active_vote_accounts(&self) -> Result<Vec<VoteAccount>, RpcError> {
        let response: VoteAccountsValue = self
            .request(GET_VOTE_ACCOUNTS, json!([{"commitment": "confirmed"}]))
            .await?;
        response
            .current
            .into_iter()
            .map(|account| {
                let vote_pubkey = Pubkey::from_str(&account.vote_pubkey).map_err(|error| {
                    RpcError::invalid_response(
                        GET_VOTE_ACCOUNTS,
                        format!("invalid vote account pubkey: {error}"),
                    )
                })?;
                Ok(VoteAccount {
                    vote_pubkey,
                    activated_stake: account.activated_stake,
                })
            })
            .collect()
    }

    async fn time_travel_to_epoch(&self, epoch: u64) -> Result<EpochInfo, RpcError> {
        let value: EpochInfoValue = self
            .request(TIME_TRAVEL, json!([{"absoluteEpoch": epoch}]))
            .await?;
        Ok(value.into())
    }

    async fn time_travel_to_slot(&self, slot: u64) -> Result<EpochInfo, RpcError> {
        let value: EpochInfoValue = self
            .request(TIME_TRAVEL, json!([{"absoluteSlot": slot}]))
            .await?;
        Ok(value.into())
    }

    async fn local_signatures(&self, limit: u64) -> Result<Vec<LocalSignatureRecord>, RpcError> {
        let response: ContextResponse<Vec<LocalSignatureValue>> =
            self.request(GET_LOCAL_SIGNATURES, json!([limit])).await?;
        response
            .value
            .into_iter()
            .map(|record| {
                let signature = Signature::from_str(&record.signature).map_err(|error| {
                    RpcError::invalid_response(
                        GET_LOCAL_SIGNATURES,
                        format!("invalid local signature: {error}"),
                    )
                })?;
                Ok(LocalSignatureRecord {
                    signature,
                    error: record.error,
                    logs: record.logs,
                })
            })
            .collect()
    }

    async fn simulate_transaction(&self, transaction: &[u8]) -> Result<RpcSimulation, RpcError> {
        let encoded = BASE64_STANDARD.encode(transaction);
        let result: ContextResponse<SimulationValue> = self
            .request(
                SIMULATE_TRANSACTION,
                json!([
                    encoded,
                    {
                        "commitment": "processed",
                        "encoding": "base64",
                        "replaceRecentBlockhash": false,
                        "sigVerify": true
                    }
                ]),
            )
            .await?;
        Ok(RpcSimulation {
            context_slot: result.context.slot,
            error: result.value.error,
            logs: result.value.logs.unwrap_or_default(),
            units_consumed: result.value.units_consumed,
        })
    }

    async fn send_transaction(&self, transaction: &[u8]) -> Result<Signature, RpcError> {
        let encoded = BASE64_STANDARD.encode(transaction);
        let signature: String = self
            .request(
                SEND_TRANSACTION,
                json!([
                    encoded,
                    {
                        "encoding": "base64",
                        "maxRetries": 0,
                        "preflightCommitment": "processed",
                        "skipPreflight": false
                    }
                ]),
            )
            .await?;
        Signature::from_str(&signature).map_err(|error| {
            RpcError::invalid_response(SEND_TRANSACTION, format!("invalid signature: {error}"))
        })
    }

    async fn signature_status(
        &self,
        signature: &Signature,
    ) -> Result<Option<SignatureStatus>, RpcError> {
        let result: ContextResponse<Vec<Option<SignatureStatusValue>>> = self
            .request(
                GET_SIGNATURE_STATUSES,
                json!([[signature.to_string()], {"searchTransactionHistory": true}]),
            )
            .await?;
        let value = result.value.into_iter().next().ok_or_else(|| {
            RpcError::invalid_response(GET_SIGNATURE_STATUSES, "status array was empty")
        })?;
        Ok(value.map(|status| SignatureStatus {
            slot: status.slot,
            confirmations: status.confirmations,
            error: status.error,
            confirmation_status: status.confirmation_status,
        }))
    }

    async fn transaction(
        &self,
        signature: &Signature,
    ) -> Result<Option<TransactionRecord>, RpcError> {
        let result: Option<TransactionValue> = self
            .request(
                GET_TRANSACTION,
                json!([
                    signature.to_string(),
                    {
                        "commitment": "confirmed",
                        "encoding": "json",
                        "maxSupportedTransactionVersion": 0
                    }
                ]),
            )
            .await?;
        result.map(decode_transaction).transpose()
    }
}

/// Verified chain gateway with no constructor that accepts a public RPC URL.
#[derive(Clone, Debug)]
pub struct SurfpoolGateway {
    rpc: JsonRpcClient,
    identity: SurfpoolIdentity,
}

impl SurfpoolGateway {
    /// Connect, verify Surfpool-specific identity, and only then return a gateway.
    ///
    /// # Errors
    ///
    /// Returns [`RpcError`] when transport fails or either identity probe is absent, malformed,
    /// or reports a Surfpool version different from [`EXPECTED_SURFPOOL_VERSION`].
    pub async fn connect(endpoint: SurfpoolRpcUrl) -> Result<Self, RpcError> {
        let rpc = JsonRpcClient::new(endpoint)?;
        let identity = rpc.version().await?;
        rpc.verify_surfnet_info().await?;
        Ok(Self { rpc, identity })
    }

    /// Repeat both Surfpool identity probes against the connected endpoint.
    ///
    /// # Errors
    ///
    /// Returns [`RpcError`] when the endpoint no longer proves the expected Surfpool identity.
    pub async fn doctor(&self) -> Result<SurfpoolIdentity, RpcError> {
        let identity = self.rpc.version().await?;
        self.rpc.verify_surfnet_info().await?;
        Ok(identity)
    }

    /// Return the identity proven before construction.
    #[must_use]
    pub const fn identity(&self) -> &SurfpoolIdentity {
        &self.identity
    }

    /// Return the validated local RPC URL.
    #[must_use]
    pub const fn endpoint(&self) -> &SurfpoolRpcUrl {
        &self.rpc.endpoint
    }

    /// Fetch a confirmed recent blockhash from Surfpool.
    ///
    /// # Errors
    ///
    /// Returns [`RpcError`] for transport, JSON-RPC, or response decoding failures.
    pub async fn latest_blockhash(&self) -> Result<LatestBlockhash, RpcError> {
        self.rpc.latest_blockhash().await
    }

    /// Read a native account balance from Surfpool.
    ///
    /// # Errors
    ///
    /// Returns [`RpcError`] for transport, JSON-RPC, or response decoding failures.
    pub async fn balance(&self, address: &Pubkey) -> Result<u64, RpcError> {
        self.rpc.balance(address).await
    }

    /// Read the confirmed block height used for blockhash-expiry reconciliation.
    ///
    /// # Errors
    ///
    /// Returns [`RpcError`] for transport, JSON-RPC, or response decoding failures.
    pub async fn block_height(&self) -> Result<u64, RpcError> {
        self.rpc.block_height().await
    }

    /// Read and decode an account from Surfpool.
    ///
    /// # Errors
    ///
    /// Returns [`RpcError`] for transport, JSON-RPC, or invalid owner/base64 response data.
    pub async fn account(&self, address: &Pubkey) -> Result<Option<AccountInfo>, RpcError> {
        self.rpc.account(address).await
    }

    /// Return the exact Surfpool rent-exempt minimum for an account data length.
    ///
    /// # Errors
    ///
    /// Returns [`RpcError`] for transport, JSON-RPC, or response decoding failures.
    pub async fn minimum_balance_for_rent_exemption(
        &self,
        data_len: usize,
    ) -> Result<u64, RpcError> {
        self.rpc.minimum_balance_for_rent_exemption(data_len).await
    }

    /// Return Surfpool's current epoch and absolute slot.
    ///
    /// # Errors
    ///
    /// Returns [`RpcError`] for transport, JSON-RPC, or response decoding failures.
    pub async fn epoch_info(&self) -> Result<EpochInfo, RpcError> {
        self.rpc.epoch_info().await
    }

    /// Return Surfpool's current native stake minimum delegation.
    ///
    /// # Errors
    ///
    /// Returns [`RpcError`] for transport, JSON-RPC, or response decoding failures.
    pub async fn stake_minimum_delegation(&self) -> Result<u64, RpcError> {
        self.rpc.stake_minimum_delegation().await
    }

    /// Return active validator vote accounts visible through the Surfpool mainnet fork.
    ///
    /// # Errors
    ///
    /// Returns [`RpcError`] for transport, JSON-RPC, or invalid vote-account data.
    pub async fn active_vote_accounts(&self) -> Result<Vec<VoteAccount>, RpcError> {
        self.rpc.active_vote_accounts().await
    }

    /// Advance the verified local Surfpool clock to an absolute future epoch.
    ///
    /// # Errors
    ///
    /// Returns [`RpcError`] when Surfpool rejects backward travel or cannot update its clock.
    pub async fn time_travel_to_epoch(&self, epoch: u64) -> Result<EpochInfo, RpcError> {
        self.rpc.time_travel_to_epoch(epoch).await
    }

    /// Advance the verified local Surfpool clock to an absolute future slot.
    ///
    /// # Errors
    ///
    /// Returns [`RpcError`] when Surfpool rejects backward travel or cannot update its clock.
    pub async fn time_travel_to_slot(&self, slot: u64) -> Result<EpochInfo, RpcError> {
        self.rpc.time_travel_to_slot(slot).await
    }

    /// Return the most recent transactions executed locally by Surfpool.
    ///
    /// # Errors
    ///
    /// Returns [`RpcError`] for transport, JSON-RPC, or invalid signature data.
    pub async fn local_signatures(
        &self,
        limit: u64,
    ) -> Result<Vec<LocalSignatureRecord>, RpcError> {
        self.rpc.local_signatures(limit).await
    }

    /// Simulate signed wire bytes locally with signature verification enabled.
    ///
    /// # Errors
    ///
    /// Returns [`RpcError`] for transport, JSON-RPC, or response decoding failures.
    pub async fn simulate_transaction(
        &self,
        transaction: &[u8],
    ) -> Result<RpcSimulation, RpcError> {
        self.rpc.simulate_transaction(transaction).await
    }

    /// Submit signed wire bytes only to the verified local Surfpool endpoint.
    ///
    /// # Errors
    ///
    /// Returns a classified [`RpcError`]. Failures after the request may have reached Surfpool are
    /// classified as an unknown outcome and must be reconciled by local signature.
    pub async fn send_transaction(&self, transaction: &[u8]) -> Result<Signature, RpcError> {
        self.rpc.send_transaction(transaction).await
    }

    /// Read a local transaction signature status.
    ///
    /// # Errors
    ///
    /// Returns [`RpcError`] for transport, JSON-RPC, or response decoding failures.
    pub async fn signature_status(
        &self,
        signature: &Signature,
    ) -> Result<Option<SignatureStatus>, RpcError> {
        self.rpc.signature_status(signature).await
    }

    /// Read a confirmed local transaction and its execution metadata.
    ///
    /// # Errors
    ///
    /// Returns [`RpcError`] for transport, JSON-RPC, or response decoding failures.
    pub async fn transaction(
        &self,
        signature: &Signature,
    ) -> Result<Option<TransactionRecord>, RpcError> {
        self.rpc.transaction(signature).await
    }
}

#[async_trait::async_trait]
impl ChainGateway for SurfpoolGateway {
    async fn simulate(&self, transaction: &[u8]) -> Result<SimulationReceipt, CookerError> {
        let simulation = self.simulate_transaction(transaction).await?;
        Ok(SimulationReceipt {
            succeeded: simulation.error.is_none(),
            units_consumed: simulation.units_consumed,
            logs: simulation.logs,
            error: simulation.error.map(|error| sanitize_json(&error)),
        })
    }

    async fn submit(&self, transaction: &[u8]) -> Result<String, CookerError> {
        let signed: VersionedTransaction = bincode::deserialize(transaction)
            .map_err(|error| CookerError::Codec(format!("invalid signed transaction: {error}")))?;
        let expected = signed
            .signatures
            .first()
            .ok_or_else(|| CookerError::Codec("signed transaction had no signature".to_owned()))?;
        let returned = self.send_transaction(transaction).await?;
        if &returned != expected {
            return Err(RpcError::invalid_response(
                SEND_TRANSACTION,
                "Surfpool returned a signature different from the signed transaction",
            )
            .into());
        }
        Ok(returned.to_string())
    }

    async fn observe(
        &self,
        action_id: &ActionId,
        signature: &str,
    ) -> Result<ChainReceipt, CookerError> {
        let signature = Signature::from_str(signature)
            .map_err(|error| CookerError::Codec(format!("invalid signature: {error}")))?;
        let status = self.signature_status(&signature).await?;
        let Some(status) = status else {
            return Ok(ChainReceipt {
                action_id: action_id.clone(),
                signature: signature.to_string(),
                status: ConfirmationStatus::Missing,
                slot: None,
                observed_at: Utc::now(),
                postconditions_met: false,
                observations: Vec::new(),
                error: None,
            });
        };
        let mut confirmation = if status.error.is_some() {
            ConfirmationStatus::Failed
        } else {
            match status.confirmation_status.as_deref() {
                Some("finalized") => ConfirmationStatus::Finalized,
                Some("confirmed") => ConfirmationStatus::Confirmed,
                Some("processed") => ConfirmationStatus::Processed,
                _ if status.confirmations.is_none() => ConfirmationStatus::Finalized,
                _ => ConfirmationStatus::Processed,
            }
        };
        let transaction = if matches!(
            confirmation,
            ConfirmationStatus::Confirmed
                | ConfirmationStatus::Finalized
                | ConfirmationStatus::Failed
        ) {
            self.transaction(&signature).await?
        } else {
            None
        };
        if transaction.is_none()
            && matches!(
                confirmation,
                ConfirmationStatus::Confirmed | ConfirmationStatus::Finalized
            )
        {
            confirmation = ConfirmationStatus::Processed;
        }
        let slot = Some(
            transaction
                .as_ref()
                .map_or(status.slot, |record| record.slot),
        );
        let error = status.error.as_ref().map(sanitize_json).or_else(|| {
            transaction
                .as_ref()
                .and_then(|record| record.error.as_ref())
                .map(sanitize_json)
        });
        Ok(ChainReceipt {
            action_id: action_id.clone(),
            signature: signature.to_string(),
            status: confirmation,
            slot,
            observed_at: Utc::now(),
            postconditions_met: false,
            observations: Vec::new(),
            error,
        })
    }

    async fn native_balance(&self, address: &str) -> Result<u64, CookerError> {
        let address = Pubkey::from_str(address)
            .map_err(|error| CookerError::Codec(format!("invalid account address: {error}")))?;
        Ok(self.balance(&address).await?)
    }

    async fn block_height(&self) -> Result<u64, CookerError> {
        Ok(Self::block_height(self).await?)
    }
}

#[derive(Debug, Deserialize)]
struct VersionResponse {
    #[serde(rename = "surfnet-version")]
    surfnet_version: Option<String>,
    #[serde(rename = "solana-core")]
    solana_core: Option<String>,
    #[serde(rename = "feature-set")]
    feature_set: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct SurfnetInfoEnvelope {
    value: Value,
}

#[derive(Debug, Deserialize)]
struct RpcContext {
    slot: u64,
}

#[derive(Debug, Deserialize)]
struct ContextResponse<T> {
    context: RpcContext,
    value: T,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LatestBlockhashValue {
    blockhash: String,
    last_valid_block_height: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EpochInfoValue {
    epoch: u64,
    slot_index: u64,
    slots_in_epoch: u64,
    absolute_slot: u64,
    block_height: u64,
    transaction_count: Option<u64>,
}

impl From<EpochInfoValue> for EpochInfo {
    fn from(value: EpochInfoValue) -> Self {
        Self {
            epoch: value.epoch,
            slot_index: value.slot_index,
            slots_in_epoch: value.slots_in_epoch,
            absolute_slot: value.absolute_slot,
            block_height: value.block_height,
            transaction_count: value.transaction_count,
        }
    }
}

#[derive(Debug, Deserialize)]
struct VoteAccountsValue {
    current: Vec<VoteAccountValue>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VoteAccountValue {
    vote_pubkey: String,
    activated_stake: u64,
}

#[derive(Debug, Deserialize)]
struct LocalSignatureValue {
    signature: String,
    #[serde(rename = "err")]
    error: Option<Value>,
    logs: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SimulationValue {
    #[serde(rename = "err")]
    error: Option<Value>,
    logs: Option<Vec<String>>,
    units_consumed: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SignatureStatusValue {
    slot: u64,
    confirmations: Option<u64>,
    #[serde(rename = "err")]
    error: Option<Value>,
    confirmation_status: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TransactionValue {
    slot: u64,
    block_time: Option<i64>,
    meta: TransactionMeta,
    transaction: JsonTransaction,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TransactionMeta {
    #[serde(rename = "err")]
    error: Option<Value>,
    fee: u64,
    log_messages: Option<Vec<String>>,
    pre_balances: Vec<u64>,
    post_balances: Vec<u64>,
    pre_token_balances: Option<Vec<TokenBalanceValue>>,
    post_token_balances: Option<Vec<TokenBalanceValue>>,
    loaded_addresses: Option<LoadedAddressesValue>,
}

#[derive(Debug, Deserialize)]
struct JsonTransaction {
    message: JsonTransactionMessage,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsonTransactionMessage {
    account_keys: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LoadedAddressesValue {
    writable: Vec<String>,
    readonly: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TokenBalanceValue {
    account_index: usize,
    mint: String,
    owner: Option<String>,
    program_id: Option<String>,
    ui_token_amount: UiTokenAmountValue,
}

#[derive(Debug, Deserialize)]
struct UiTokenAmountValue {
    amount: String,
    decimals: u8,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AccountValue {
    lamports: u64,
    owner: String,
    executable: bool,
    rent_epoch: u64,
    data: Value,
}

fn decode_account(context_slot: u64, value: &AccountValue) -> Result<AccountInfo, RpcError> {
    let owner = Pubkey::from_str(&value.owner).map_err(|error| {
        RpcError::invalid_response(GET_ACCOUNT_INFO, format!("invalid account owner: {error}"))
    })?;
    let data = value.data.as_array().ok_or_else(|| {
        RpcError::invalid_response(GET_ACCOUNT_INFO, "account data was not encoded as an array")
    })?;
    if data.len() != 2 || data.get(1).and_then(Value::as_str) != Some("base64") {
        return Err(RpcError::invalid_response(
            GET_ACCOUNT_INFO,
            "account data did not use base64 encoding",
        ));
    }
    let encoded = data.first().and_then(Value::as_str).ok_or_else(|| {
        RpcError::invalid_response(GET_ACCOUNT_INFO, "account base64 payload was missing")
    })?;
    let decoded = BASE64_STANDARD.decode(encoded).map_err(|error| {
        RpcError::invalid_response(GET_ACCOUNT_INFO, format!("invalid account base64: {error}"))
    })?;
    Ok(AccountInfo {
        context_slot,
        lamports: value.lamports,
        owner,
        executable: value.executable,
        rent_epoch: value.rent_epoch,
        data: decoded,
    })
}

fn decode_transaction(value: TransactionValue) -> Result<TransactionRecord, RpcError> {
    let mut account_keys = decode_pubkeys(
        value.transaction.message.account_keys,
        "transaction account key",
    )?;
    if let Some(loaded) = value.meta.loaded_addresses {
        account_keys.extend(decode_pubkeys(
            loaded.writable,
            "loaded writable account key",
        )?);
        account_keys.extend(decode_pubkeys(
            loaded.readonly,
            "loaded readonly account key",
        )?);
    }
    let pre_token_balances = decode_token_balances(value.meta.pre_token_balances)?;
    let post_token_balances = decode_token_balances(value.meta.post_token_balances)?;
    Ok(TransactionRecord {
        slot: value.slot,
        block_time: value.block_time,
        error: value.meta.error,
        fee: value.meta.fee,
        logs: value.meta.log_messages.unwrap_or_default(),
        pre_balances: value.meta.pre_balances,
        post_balances: value.meta.post_balances,
        account_keys,
        pre_token_balances,
        post_token_balances,
    })
}

fn decode_pubkeys(values: Vec<String>, label: &str) -> Result<Vec<Pubkey>, RpcError> {
    values
        .into_iter()
        .map(|value| {
            Pubkey::from_str(&value).map_err(|error| {
                RpcError::invalid_response(GET_TRANSACTION, format!("invalid {label}: {error}"))
            })
        })
        .collect()
}

fn decode_token_balances(
    values: Option<Vec<TokenBalanceValue>>,
) -> Result<Vec<TokenBalanceRecord>, RpcError> {
    values
        .unwrap_or_default()
        .into_iter()
        .map(|value| {
            let mint = decode_optional_pubkey(Some(value.mint), "token balance mint")?
                .ok_or_else(|| RpcError::invalid_response(GET_TRANSACTION, "token mint missing"))?;
            let owner = decode_optional_pubkey(value.owner, "token balance owner")?;
            let program_id = decode_optional_pubkey(value.program_id, "token balance program id")?;
            let amount = value.ui_token_amount.amount.parse().map_err(|error| {
                RpcError::invalid_response(
                    GET_TRANSACTION,
                    format!("invalid raw token balance: {error}"),
                )
            })?;
            Ok(TokenBalanceRecord {
                account_index: value.account_index,
                mint,
                owner,
                program_id,
                amount,
                decimals: value.ui_token_amount.decimals,
            })
        })
        .collect()
}

fn decode_optional_pubkey(value: Option<String>, label: &str) -> Result<Option<Pubkey>, RpcError> {
    value
        .map(|value| {
            Pubkey::from_str(&value).map_err(|error| {
                RpcError::invalid_response(GET_TRANSACTION, format!("invalid {label}: {error}"))
            })
        })
        .transpose()
}

fn sanitize_json(value: &Value) -> String {
    value.to_string().chars().take(512).collect()
}

#[cfg(test)]
mod tests {
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_partial_json, method},
    };

    use super::*;
    use crate::RpcFailureClass;

    fn endpoint(server: &MockServer) -> Result<SurfpoolRpcUrl, RpcError> {
        server.uri().parse()
    }

    #[tokio::test]
    async fn connect_rejects_generic_local_json_rpc() -> Result<(), Box<dyn std::error::Error>> {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_partial_json(json!({"method": GET_VERSION})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": {"solana-core": "4.0.0"}
            })))
            .mount(&server)
            .await;

        let error = SurfpoolGateway::connect(endpoint(&server)?).await;
        let error = error
            .err()
            .ok_or("generic JSON-RPC unexpectedly passed identity check")?;
        assert_eq!(error.class(), RpcFailureClass::Identity);
        Ok(())
    }

    #[tokio::test]
    async fn connect_requires_surfnet_info_after_version() -> Result<(), Box<dyn std::error::Error>>
    {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_partial_json(json!({"method": GET_VERSION})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": {
                    "surfnet-version": EXPECTED_SURFPOOL_VERSION,
                    "solana-core": "4.0.0"
                }
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_partial_json(json!({"method": GET_SURFNET_INFO})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "jsonrpc": "2.0",
                "id": 2,
                "error": {"code": -32601, "message": "method not found"}
            })))
            .mount(&server)
            .await;

        let error = SurfpoolGateway::connect(endpoint(&server)?).await;
        let error = error
            .err()
            .ok_or("missing Surfpool info method unexpectedly passed")?;
        assert_eq!(error.class(), RpcFailureClass::Identity);
        Ok(())
    }

    #[tokio::test]
    async fn http_failure_after_send_is_unknown_outcome() -> Result<(), Box<dyn std::error::Error>>
    {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_partial_json(json!({"method": SEND_TRANSACTION})))
            .respond_with(ResponseTemplate::new(503).set_body_string("response lost after submit"))
            .mount(&server)
            .await;
        let rpc = JsonRpcClient::new(endpoint(&server)?)?;
        let error = rpc
            .send_transaction(&[1, 2, 3])
            .await
            .err()
            .ok_or("send unexpectedly succeeded")?;
        assert_eq!(error.class(), RpcFailureClass::UnknownOutcome);
        Ok(())
    }
}

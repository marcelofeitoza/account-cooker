#!/usr/bin/env bash
#
# Bounded native-transfer run against public Solana devnet.
#
# This is the public-network counterpart to scripts/surfpool-soak.sh. It writes real
# transactions to a real cluster and spends real devnet SOL. It is deliberately opt-in and
# never runs as part of scripts/full-demo.sh.
#
# Funding, once, before the first run:
#
#   solana-keygen new --no-bip39-passphrase --outfile .devnet/keys/payer.json
#   chmod 600 .devnet/keys/payer.json
#   solana transfer --url https://api.devnet.solana.com --allow-unfunded-recipient \
#     "$(solana-keygen pubkey .devnet/keys/payer.json)" 1
#
# Fund exactly one payer and let it fund anything else by system transfer. The devnet faucet
# is rate limited per source address, so airdropping per key fails quickly.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

RPC_URL="${COOKER_DEVNET_RPC_URL:-https://api.devnet.solana.com}"
DEVNET_GENESIS_HASH="EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG"
TRANSFER_LAMPORTS=1000000
MAX_FEE_LAMPORTS=10000
RESERVE_LAMPORTS=100000000

transactions=200
# The default allows bounded overlap while the public gateway applies its configured pacing.
# Reported throughput remains an end-to-end observation; this run does not isolate a bottleneck.
concurrency=8
signer_path="${PROJECT_ROOT}/.devnet/keys/payer.json"
output_dir="${PROJECT_ROOT}/evidence/raw/devnet-soak/$(date -u '+%Y%m%dT%H%M%SZ')-$$"
promote_dir="${PROJECT_ROOT}/evidence/devnet"
promote=0

die() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

note() {
  printf '%s\n' "$*"
}

is_uint() {
  [[ "$1" =~ ^[0-9]+$ ]]
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"
}

while (($#)); do
  case "$1" in
    --transactions)
      [[ $# -ge 2 ]] || die "--transactions requires a value"
      transactions="$2"
      shift 2
      ;;
    --concurrency)
      [[ $# -ge 2 ]] || die "--concurrency requires a value"
      concurrency="$2"
      shift 2
      ;;
    --signer)
      [[ $# -ge 2 ]] || die "--signer requires a path"
      signer_path="$2"
      shift 2
      ;;
    --output-dir)
      [[ $# -ge 2 ]] || die "--output-dir requires a path"
      output_dir="$2"
      shift 2
      ;;
    --promote)
      promote=1
      shift
      ;;
    *)
      die "usage: $0 [--transactions COUNT] [--concurrency N] [--signer PATH] [--output-dir PATH] [--promote]"
      ;;
  esac
done

is_uint "${transactions}" || die "transaction count must be an integer"
is_uint "${concurrency}" || die "concurrency must be an integer"
((transactions >= 1)) || die "transaction count must be positive"
((concurrency >= 1 && concurrency <= 32)) || die "concurrency must be in 1..=32"

case "${output_dir}" in
  /*) ;;
  *) output_dir="${PROJECT_ROOT}/${output_dir}" ;;
esac
[[ ! -e "${output_dir}" ]] || die "output directory already exists: ${output_dir}"

require_command cargo
require_command curl
require_command jq
require_command tee

[[ -f "${signer_path}" ]] || die "devnet payer keypair is missing: ${signer_path}"

rpc_call() {
  curl -sS --max-time 30 -X POST "${RPC_URL}" \
    -H 'Content-Type: application/json' \
    -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$1\",\"params\":$2}"
}

genesis_hash="$(rpc_call getGenesisHash '[]' | jq -r '.result // empty')"
[[ "${genesis_hash}" == "${DEVNET_GENESIS_HASH}" ]] ||
  die "endpoint ${RPC_URL} reported genesis ${genesis_hash:-<none>}, expected devnet ${DEVNET_GENESIS_HASH}"

payer="$(cargo run --locked --quiet -p account-cooker -- keygen \
  --project-root "${PROJECT_ROOT}" --output "${signer_path}" | awk '{print $2}')"
[[ -n "${payer}" ]] || die "could not resolve the devnet payer address"

balance="$(rpc_call getBalance "[\"${payer}\",{\"commitment\":\"confirmed\"}]" |
  jq -r '.result.value // empty')"
is_uint "${balance}" || die "could not read the devnet payer balance"
required=$((transactions * (TRANSFER_LAMPORTS + MAX_FEE_LAMPORTS) + RESERVE_LAMPORTS))
((balance >= required)) ||
  die "payer ${payer} holds ${balance} lamports, run needs ${required}; fund it with a system transfer"

note "Bounded devnet native-transfer run"
note "  endpoint:     ${RPC_URL}"
note "  genesis:      ${genesis_hash}"
note "  payer:        ${payer}"
note "  balance:      ${balance} lamports"
note "  transactions: ${transactions}"
note "  concurrency:  ${concurrency}"
note "  evidence:     ${output_dir}"

mkdir -p "${output_dir}"

COOKER_DEVNET_TRANSACTIONS="${transactions}" \
COOKER_DEVNET_CONCURRENCY="${concurrency}" \
COOKER_DEVNET_SIGNER_PATH="${signer_path}" \
COOKER_DEVNET_RPC_URL="${RPC_URL}" \
COOKER_DEVNET_DATABASE="${output_dir}/cooker.sqlite" \
COOKER_DEVNET_EVIDENCE="${output_dir}/devnet-soak.json" \
  cargo test --locked -p cooker-solana --test devnet_soak \
    bounded_devnet_run_confirms_and_reconciles_without_duplicate_intents \
    -- --ignored --exact --nocapture 2>&1 | tee "${output_dir}/test.log"

# Gate the engine invariants that must hold on any network. Confirmation rate is measured and
# reported, never asserted, because inclusion is the cluster's decision and not the engine's.
jq -e --argjson expected "${transactions}" '
  .schema_version == 2
  and .scenario == "bounded_devnet_native_transfer_run"
  and .network.cluster == "solana-devnet"
  and .network.genesis_hash == "'"${DEVNET_GENESIS_HASH}"'"
  and .transaction_count == $expected
  and .logical_action_count == $expected
  and (.confirmed_action_count + .unconfirmed_action_count) == $expected
  and .duplicate_logical_intents == 0
  and .duplicate_signatures == 0
  and .budget_violations == 0
  and .unresolved_submitted == 0
  and .unresolved_unknown == 0
  and .response_loss_injections == 1
  and .reconciled_to_confirmed >= 1
  and .in_process_component_reconstructions == 1
  and .component_reconstruction.os_process_restarts == 0
  and .component_reconstruction.runtime_engine_rebuilt == true
  and .component_reconstruction.sqlite_database_reopened == true
  and .component_reconstruction.signer_reloaded == true
  and .component_reconstruction.gateway_reused == true
  and .exact_source_debit_count == .confirmed_action_count
  and .exact_destination_credit_count == .confirmed_action_count
  and .state_delta.payer_equation_met == true
  and .peak_worker_count <= .max_concurrency
  and .signature_order == "planned_sequence"
  and (.signatures | length) == $expected
  and ([.signatures[].sequence] == [range(0; $expected)])
  and ([.unconfirmed_causes[]] | add // 0) == .unconfirmed_action_count
  and (.unconfirmed_causes | has("unclassified") | not)
' "${output_dir}/devnet-soak.json" >/dev/null || die "devnet run evidence gate failed"

confirmed="$(jq -r '.confirmed_action_count' "${output_dir}/devnet-soak.json")"
unconfirmed="$(jq -r '.unconfirmed_action_count' "${output_dir}/devnet-soak.json")"
rate="$(jq -r '.confirmed_per_second' "${output_dir}/devnet-soak.json")"
note "Devnet run: passed (${confirmed} confirmed, ${unconfirmed} unconfirmed, ${rate} confirmed/s)"
note "Evidence: ${output_dir}/devnet-soak.json"

if ((promote)); then
  mkdir -p "${promote_dir}"
  cp "${output_dir}/devnet-soak.json" "${promote_dir}/devnet-soak.json"
  (
    cd "${promote_dir}"
    if command -v sha256sum >/dev/null 2>&1; then
      sha256sum devnet-soak.json >checksums.txt
    else
      shasum -a 256 devnet-soak.json >checksums.txt
    fi
  )
  note "Promoted committed record: ${promote_dir}/devnet-soak.json"
fi
note "Verify any signature with:"
note "  https://explorer.solana.com/tx/<signature>?cluster=devnet"

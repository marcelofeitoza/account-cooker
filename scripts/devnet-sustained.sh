#!/usr/bin/env bash
#
# Sustained native-transfer run against public Solana devnet.
#
# scripts/devnet-soak.sh proves the engine's invariants inside one short bounded window on a
# public cluster. This script runs the same engine for hours and records a measurement after
# every round, so duration itself is the result. It writes real transactions to a real cluster
# and spends real devnet SOL. It is opt-in and never runs as part of scripts/full-demo.sh.
#
# The run outlives this shell only if you detach it. It is designed for that: every round
# appends a whole line to the JSON Lines ledger and rewrites a self-consistent aggregate, so a
# run that is killed at any point still leaves evidence describing exactly the rounds that
# finished.
#
#   nohup ./scripts/devnet-sustained.sh --output-dir evidence/raw/devnet-sustained/run \
#     > /tmp/devnet-sustained.log 2>&1 &
#
# Funding, once, before the first run:
#
#   solana-keygen new --no-bip39-passphrase --outfile .devnet/keys/payer.json
#   chmod 600 .devnet/keys/payer.json
#   solana transfer --url https://api.devnet.solana.com --allow-unfunded-recipient \
#     "$(solana-keygen pubkey .devnet/keys/payer.json)" 1
#
# Fund exactly one payer and let it fund anything else by system transfer. The devnet faucet is
# rate limited per source address, so airdropping per key fails quickly.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

RPC_URL="${COOKER_SUSTAINED_RPC_URL:-https://api.devnet.solana.com}"
DEVNET_GENESIS_HASH="EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG"
TOPUP_LAMPORTS=1000
MAX_FEE_LAMPORTS=10000
# Kept in step with RUN_BALANCE_FLOOR_LAMPORTS in the test.
BALANCE_FLOOR_LAMPORTS=400000000

duration_secs=21600
round_interval_secs=60
round_batch=4
pool_size=16
signer_path="${PROJECT_ROOT}/.devnet/keys/payer.json"
output_dir="${PROJECT_ROOT}/evidence/raw/devnet-sustained/$(date -u '+%Y%m%dT%H%M%SZ')-$$"
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
    --duration-secs)
      [[ $# -ge 2 ]] || die "--duration-secs requires a value"
      duration_secs="$2"
      shift 2
      ;;
    --round-interval-secs)
      [[ $# -ge 2 ]] || die "--round-interval-secs requires a value"
      round_interval_secs="$2"
      shift 2
      ;;
    --round-batch)
      [[ $# -ge 2 ]] || die "--round-batch requires a value"
      round_batch="$2"
      shift 2
      ;;
    --pool-size)
      [[ $# -ge 2 ]] || die "--pool-size requires a value"
      pool_size="$2"
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
      die "usage: $0 [--duration-secs N] [--round-interval-secs N] [--round-batch N]
       [--pool-size N] [--signer PATH] [--output-dir PATH] [--promote]"
      ;;
  esac
done

for value in "${duration_secs}" "${round_interval_secs}" "${round_batch}" "${pool_size}"; do
  is_uint "${value}" || die "run dimensions must be integers"
done
((round_interval_secs >= 10)) || die "round interval must be at least 10 seconds"
((duration_secs >= round_interval_secs)) || die "duration must cover at least one round"
((round_batch >= 1 && round_batch <= 16)) || die "round batch must be in 1..=16"
((pool_size >= 1 && pool_size <= 64)) || die "destination pool must be in 1..=64"

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

create_lamports="$(rpc_call getMinimumBalanceForRentExemption '[0,{"commitment":"confirmed"}]' |
  jq -r '.result // empty')"
is_uint "${create_lamports}" || die "could not read the devnet rent-exempt minimum"

payer="$(cargo run --locked --quiet -p account-cooker -- keygen \
  --project-root "${PROJECT_ROOT}" --output "${signer_path}" | awk '{print $2}')"
[[ -n "${payer}" ]] || die "could not resolve the devnet payer address"

balance="$(rpc_call getBalance "[\"${payer}\",{\"commitment\":\"confirmed\"}]" |
  jq -r '.result.value // empty')"
is_uint "${balance}" || die "could not read the devnet payer balance"

steady_rounds=$((duration_secs / round_interval_secs))
((steady_rounds >= 1)) || die "duration must cover at least one steady round"
steady_actions=$((steady_rounds * round_batch))
planned_actions=$((steady_actions + pool_size))
# Budget against the fee ceiling, not the observed fee. The payer cannot be topped up.
required=$((pool_size * create_lamports + steady_actions * TOPUP_LAMPORTS +
  planned_actions * MAX_FEE_LAMPORTS + BALANCE_FLOOR_LAMPORTS))
((balance >= required)) ||
  die "payer ${payer} holds ${balance} lamports, plan needs ${required} including the \
${BALANCE_FLOOR_LAMPORTS} lamport floor"

note "Sustained devnet native-transfer run"
note "  endpoint:      ${RPC_URL}"
note "  genesis:       ${genesis_hash}"
note "  payer:         ${payer}"
note "  balance:       ${balance} lamports"
note "  duration:      ${duration_secs} s"
note "  round every:   ${round_interval_secs} s"
note "  round batch:   ${round_batch}"
note "  pool size:     ${pool_size}"
note "  planned:       ${planned_actions} actions over $((steady_rounds + 1)) rounds"
note "  ceiling spend: $((required - BALANCE_FLOOR_LAMPORTS)) lamports"
note "  evidence:      ${output_dir}"

mkdir -p "${output_dir}"

COOKER_SUSTAINED_DURATION_SECS="${duration_secs}" \
COOKER_SUSTAINED_ROUND_INTERVAL_SECS="${round_interval_secs}" \
COOKER_SUSTAINED_ROUND_BATCH="${round_batch}" \
COOKER_SUSTAINED_POOL_SIZE="${pool_size}" \
COOKER_SUSTAINED_SIGNER_PATH="${signer_path}" \
COOKER_SUSTAINED_RPC_URL="${RPC_URL}" \
COOKER_SUSTAINED_DATABASE="${output_dir}/cooker.sqlite" \
COOKER_SUSTAINED_EVIDENCE="${output_dir}/sustained-soak.json" \
COOKER_SUSTAINED_ROUNDS="${output_dir}/sustained-rounds.jsonl" \
  cargo test --locked -p cooker-solana --test devnet_sustained \
    sustained_devnet_run_holds_engine_invariants_across_hours \
    -- --ignored --exact --nocapture 2>&1 | tee "${output_dir}/test.log"

# Gate the engine invariants that must hold on any network for however long it runs. Inclusion
# rate, throughput, growth, and drift are measured and reported, never asserted: they are the
# cluster's and the host's answers, not the engine's promises.
jq -e '
  .schema_version == 1
  and .scenario == "sustained_devnet_native_transfer_run"
  and .completed == true
  and .network.cluster == "solana-devnet"
  and .network.genesis_hash == "'"${DEVNET_GENESIS_HASH}"'"
  and .rounds_completed >= 1
  and .executed_action_count == (.confirmed_action_count + .unconfirmed_action_count)
  and .executed_action_count == (.signatures | length)
  and .executed_action_count <= .planned_action_count
  and .duplicate_logical_intents == 0
  and .duplicate_signatures == 0
  and .budget_violations == 0
  and .unresolved_submitted == 0
  and .unresolved_unknown == 0
  and (.worker_errors | length) == 0
  and .exact_source_debit_count == .confirmed_action_count
  and .exact_destination_credit_count == .confirmed_action_count
  and .state_delta.payer_equation_met == true
  and .state_delta.payer_after_lamports >= '"${BALANCE_FLOOR_LAMPORTS}"'
  and .peak_worker_count <= .max_concurrency
  and .signature_order == "planned_sequence"
  and ([.signatures[].sequence] | sort) == ([.signatures[].sequence] | sort | unique)
  and ([.unconfirmed_causes[]] | add // 0) == .unconfirmed_action_count
  and (.unconfirmed_causes | has("unclassified") | not)
  and .chain_observation.slot_leaders.windows_measured >= 1
' "${output_dir}/sustained-soak.json" >/dev/null || die "sustained run evidence gate failed"

rounds_recorded="$(wc -l <"${output_dir}/sustained-rounds.jsonl" | tr -d ' ')"
rounds_claimed="$(jq -r '.rounds_completed' "${output_dir}/sustained-soak.json")"
[[ "${rounds_recorded}" == "${rounds_claimed}" ]] ||
  die "round ledger has ${rounds_recorded} lines but the aggregate claims ${rounds_claimed} rounds"

confirmed="$(jq -r '.confirmed_action_count' "${output_dir}/sustained-soak.json")"
unconfirmed="$(jq -r '.unconfirmed_action_count' "${output_dir}/sustained-soak.json")"
hours="$(jq -r '.wall_time_hours' "${output_dir}/sustained-soak.json")"
note "Sustained run: passed (${rounds_claimed} rounds, ${confirmed} confirmed, \
${unconfirmed} unconfirmed, ${hours} h)"
note "Evidence: ${output_dir}/sustained-soak.json"

if ((promote)); then
  mkdir -p "${promote_dir}"
  cp "${output_dir}/sustained-soak.json" "${promote_dir}/sustained-soak.json"
  cp "${output_dir}/sustained-rounds.jsonl" "${promote_dir}/sustained-rounds.jsonl"
  (
    cd "${promote_dir}"
    if command -v sha256sum >/dev/null 2>&1; then
      sha256sum devnet-soak.json sustained-soak.json sustained-rounds.jsonl >checksums.txt
    else
      shasum -a 256 devnet-soak.json sustained-soak.json sustained-rounds.jsonl >checksums.txt
    fi
  )
  note "Promoted committed record: ${promote_dir}/sustained-soak.json"
fi
note "Verify any signature with:"
note "  https://explorer.solana.com/tx/<signature>?cluster=devnet"

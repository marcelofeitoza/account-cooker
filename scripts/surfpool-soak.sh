#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib/surfpool-common.sh
source "${SCRIPT_DIR}/lib/surfpool-common.sh"

transactions=1000
output_dir="${PROJECT_ROOT}/evidence/raw/surfpool-soak/$(date -u '+%Y%m%dT%H%M%SZ')-$$"
quick=0
transactions_supplied=0
while (($#)); do
  case "$1" in
    --transactions)
      [[ $# -ge 2 ]] || surfpool_die "--transactions requires a value"
      transactions="$2"
      transactions_supplied=1
      shift 2
      ;;
    --output-dir)
      [[ $# -ge 2 ]] || surfpool_die "--output-dir requires a path"
      output_dir="$2"
      shift 2
      ;;
    --quick)
      quick=1
      shift
      ;;
    *) surfpool_die "usage: $0 [--quick] [--transactions COUNT] [--output-dir PATH]" ;;
  esac
done
if ((quick && transactions_supplied == 0)); then
  transactions=25
fi
if ((quick)); then
  concurrency="${COOKER_SOAK_CONCURRENCY:-4}"
else
  concurrency="${COOKER_SOAK_CONCURRENCY:-16}"
fi
surfpool_is_uint "${transactions}" || surfpool_die "transaction count must be an integer"
surfpool_is_uint "${concurrency}" || surfpool_die "concurrency must be an integer"
((concurrency >= 1 && concurrency <= 64)) || surfpool_die "concurrency must be in 1..=64"
if ((quick)); then
  ((transactions >= 2)) || surfpool_die "quick soak requires at least 2 transactions"
else
  ((transactions >= 1000)) || surfpool_die "acceptance soak requires at least 1000 transactions"
fi
case "${output_dir}" in
  /*) ;;
  *) output_dir="${PROJECT_ROOT}/${output_dir}" ;;
esac
[[ ! -e "${output_dir}" ]] || surfpool_die "output directory already exists: ${output_dir}"

surfpool_require_harness_commands
surfpool_require_command cargo
surfpool_require_command tee
surfpool_validate_settings
surfpool_verify_install

started_here=0
existing_pid="$(surfpool_read_pid || true)"
if [[ -z "${existing_pid}" ]] || ! surfpool_pid_matches "${existing_pid}"; then
  if [[ -n "${existing_pid}" ]] && surfpool_pid_is_live "${existing_pid}"; then
    surfpool_die "PID file points to a process outside this Surfpool harness: ${existing_pid}"
  fi
  started_here=1
  trap 'if ((started_here)); then "${SCRIPT_DIR}/surfpool-stop.sh" >/dev/null 2>&1 || true; fi' EXIT
  "${SCRIPT_DIR}/surfpool-start.sh"
fi
"${SCRIPT_DIR}/surfpool-doctor.sh"
surfpool_verify_runtime_env
# shellcheck disable=SC1090
source "${SURFPOOL_RUNTIME_ENV}"

mkdir -p "${output_dir}"
unset COOKER_SOAK_ALLOW_SHORT
if ((quick)); then
  export COOKER_SOAK_ALLOW_SHORT=1
  soak_mode=quick
else
  soak_mode=canonical
fi
COOKER_SOAK_TRANSACTIONS="${transactions}" \
COOKER_SOAK_CONCURRENCY="${concurrency}" \
COOKER_SOAK_RESTART_SURFPOOL=1 \
COOKER_SOAK_DATABASE="${output_dir}/cooker.sqlite" \
COOKER_SOAK_EVIDENCE="${output_dir}/soak.json" \
COOKER_SURFNET_ID="${COOKER_SURFNET_ID}" \
COOKER_RPC_URL="${COOKER_RPC_URL}" \
COOKER_SIGNER_PATH="${COOKER_SIGNER_PATH}" \
  cargo test --locked -p cooker-solana --test surfpool_soak \
    compressed_soak_restarts_and_reconciles_without_duplicate_intents \
    -- --ignored --exact --nocapture 2>&1 | tee "${output_dir}/test.log"

jq -e --argjson expected "${transactions}" '
  .schema_version == 1
  and .transaction_count == $expected
  and .confirmed_action_count == $expected
  and .logical_action_count == $expected
  and .failures == 0
  and .duplicate_logical_intents == 0
  and .duplicate_signatures == 0
  and .budget_violations == 0
  and .unresolved_submitted == 0
  and .unresolved_unknown == 0
  and .response_loss_injections == 1
  and .response_loss_reconciled_without_resend == 1
  and .runtime_stack_restarts == 1
  and .surfpool_process_restarts == 1
  and .peak_worker_count <= .max_concurrency
  and .state_delta.payer_debit_lamports
      == .state_delta.transferred_lamports + .state_delta.fees_lamports
' "${output_dir}/soak.json" >/dev/null || surfpool_die "Surfpool soak evidence gate failed"

jq '{
  surfpoolVersion,
  binarySha256,
  network,
  surfnetId,
  rpcUrl: "http://127.0.0.1:<local>",
  database: "persistent-local-surfnet",
  snapshotArchiveSha256,
  snapshotSha256,
  configuredAirdropLamports,
  effectiveAirdropLamports,
  resumedPersistentDatabase
}' "${SURFPOOL_SESSION_FILE}" >"${output_dir}/surfpool-session.json"

if ((started_here)); then
  "${SCRIPT_DIR}/surfpool-stop.sh"
  started_here=0
fi
surfpool_note "Compressed Surfpool soak: passed (${transactions} real local transactions, ${soak_mode})"
surfpool_note "Evidence: ${output_dir}"

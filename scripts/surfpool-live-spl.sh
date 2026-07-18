#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib/surfpool-common.sh
source "${SCRIPT_DIR}/lib/surfpool-common.sh"

output_dir="${PROJECT_ROOT}/evidence/raw/surfpool-spl/$(date -u '+%Y%m%dT%H%M%SZ')-$$"
if [[ "${1:-}" == "--output-dir" ]]; then
  [[ $# -eq 2 ]] || surfpool_die "usage: $0 [--output-dir PATH]"
  output_dir="$2"
elif [[ $# -ne 0 ]]; then
  surfpool_die "usage: $0 [--output-dir PATH]"
fi
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
COOKER_SPL_EVIDENCE="${output_dir}/spl-transfer.json" \
COOKER_RPC_URL="${COOKER_RPC_URL}" \
COOKER_SIGNER_PATH="${COOKER_SIGNER_PATH}" \
  cargo test -p cooker-solana --test surfpool_spl_live \
    classic_spl_adapter_proves_creation_transfer_and_exact_overhead \
    -- --ignored --exact --nocapture 2>&1 | tee "${output_dir}/test.log"

jq -e '
  .schema_version == 1
  and .scenario == "classic_spl_transfer_with_destination_ata_creation"
  and .postconditions_met == true
  and .source_before_raw - .source_after_raw == .transferred_raw
  and .destination_after_raw - .destination_before_raw == .transferred_raw
  and .payer_native_debit_lamports == .fee_lamports + .ata_rent_lamports
' "${output_dir}/spl-transfer.json" >/dev/null || surfpool_die "SPL evidence gate failed"

jq '{
  surfpoolVersion,
  binarySha256,
  network,
  surfnetId,
  rpcUrl: "http://127.0.0.1:<local>",
  database: "persistent-local-surfnet"
}' "${SURFPOOL_SESSION_FILE}" >"${output_dir}/surfpool-session.json"

if ((started_here)); then
  "${SCRIPT_DIR}/surfpool-stop.sh"
  started_here=0
fi
surfpool_note "Classic SPL Surfpool acceptance: passed"
surfpool_note "Evidence: ${output_dir}"

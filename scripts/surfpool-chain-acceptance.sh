#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib/surfpool-common.sh
source "${SCRIPT_DIR}/lib/surfpool-common.sh"

if (($# != 0)); then
  surfpool_die "usage: $0"
fi

surfpool_require_harness_commands
surfpool_require_command cargo
surfpool_require_command solana
surfpool_require_command solana-keygen
surfpool_validate_settings
surfpool_verify_install

evidence_log="${COOKER_CHAIN_EVIDENCE_LOG:-${SURFPOOL_HOME}/evidence/chain-acceptance-$(date -u '+%Y%m%dT%H%M%SZ').log}"
evidence_dir="$(dirname "${evidence_log}")"
mkdir -p "${evidence_dir}"
chmod 700 "${evidence_dir}"
[[ ! -e "${evidence_log}" ]] || surfpool_die "chain evidence log already exists: ${evidence_log}"
touch "${evidence_log}"
chmod 600 "${evidence_log}"

run_gate() {
  local label="$1"
  shift
  printf '\n[%s] %s\n' "$(date -u '+%Y-%m-%dT%H:%M:%SZ')" "${label}" | tee -a "${evidence_log}"
  "$@" 2>&1 | tee -a "${evidence_log}"
}

require_marker() {
  local count marker="$1"
  count="$(grep -Fc "${marker}" "${evidence_log}" || true)"
  if [[ "${count}" != "1" ]]; then
    surfpool_die "acceptance output must contain exactly one evidence marker (${count} found): ${marker}"
  fi
}

cd "${PROJECT_ROOT}"
export CARGO_TERM_COLOR=never

run_gate "Rust formatting" cargo fmt --all -- --check
run_gate "Solana adapter clippy" cargo clippy --locked -p cooker-solana --all-targets -- -D warnings
run_gate "Solana adapter unit tests" cargo test --locked -p cooker-solana --lib

started_here=0
existing_pid="$(surfpool_read_pid || true)"
if [[ -z "${existing_pid}" ]]; then
  started_here=1
fi
cleanup() {
  local status=$?
  trap - EXIT INT TERM
  if ((started_here)); then
    "${SCRIPT_DIR}/surfpool-stop.sh" >/dev/null 2>&1 || true
  fi
  exit "${status}"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

"${SCRIPT_DIR}/surfpool-start.sh" 2>&1 | tee -a "${evidence_log}"
surfpool_verify_runtime_env
set -a
# shellcheck disable=SC1090
source "${SURFPOOL_RUNTIME_ENV}"
set +a

doctor_config="${SURFPOOL_HOME}/state/chain-doctor.toml"
run_gate "Generate doctor configuration" \
  cargo run --quiet --locked -p account-cooker -- \
  init --output "${doctor_config}" --force
doctor_config_temp="${doctor_config}.tmp.$$"
awk -v rpc_url="${COOKER_RPC_URL}" -v surfnet_id="${SURFPOOL_ID}" '
  /^rpc_url = / { print "rpc_url = \"" rpc_url "\""; next }
  /^surfnet_id = / { print "surfnet_id = \"" surfnet_id "\""; next }
  { print }
' "${doctor_config}" >"${doctor_config_temp}"
chmod 600 "${doctor_config_temp}"
mv "${doctor_config_temp}" "${doctor_config}"
run_gate "Read-only cooker doctor" \
  cargo run --quiet --locked -p account-cooker -- \
  doctor --config "${doctor_config}"

run_gate "Native SOL transfer with exact fee attribution" \
  cargo test --locked -p cooker-solana --test surfpool_e2e \
  native_adapter_accepts_on_real_surfpool -- --ignored --exact --nocapture

run_gate "Jupiter exact-input swap from pinned reviewed state" \
  cargo test --locked -p cooker-solana --test jupiter_surfpool_e2e \
  jupiter_exact_input_swap_accepts_on_real_surfpool -- --ignored --exact --nocapture

run_gate "Classic SPL transfer with ATA creation" \
  cargo test --locked -p cooker-solana --test surfpool_spl_live \
  classic_spl_adapter_proves_creation_transfer_and_exact_overhead -- --ignored --nocapture

run_gate "Failure and same-signature reconciliation" \
  cargo test --locked -p cooker-solana --test surfpool_faults_live \
  stale_insufficient_simulation_and_lost_response_reconcile_on_surfpool \
  -- --ignored --nocapture

run_gate "Six real-process crash and restart checkpoints" \
  cargo test --locked -p cooker-solana --test surfpool_process_recovery \
  every_process_crash_checkpoint_recovers_on_real_surfpool \
  -- --ignored --exact --nocapture

run_gate "Native stake full lifecycle" \
  cargo test --locked -p cooker-solana --test surfpool_stake_live \
  native_stake_full_lifecycle_on_real_surfpool -- --ignored --nocapture

require_marker "COOKER_NATIVE_EVIDENCE="
require_marker "COOKER_SPL_EVIDENCE="
require_marker "COOKER_STAKE_EVIDENCE="
require_marker "COOKER_FAULT_EVIDENCE="
require_marker "COOKER_JUPITER_EVIDENCE="
require_marker "COOKER_PROCESS_RECOVERY_EVIDENCE="
require_marker '"healthy": true'
require_marker '"signer_loaded": false'
require_marker '"state_changed": false'

surfpool_note "Surfpool chain acceptance: passed"
surfpool_note "Raw local acceptance log: ${evidence_log}"

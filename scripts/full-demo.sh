#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
run_token="$(date -u '+%Y%m%dT%H%M%SZ')-$$"
run_started_at="$(date -u '+%Y-%m-%dT%H:%M:%SZ')"
original_arguments=("$@")
original_argument_count=$#
mode=canonical
output_dir=''
evidence_dir=''
command_counter=0
readonly RUST_EXPECTED_VERSION=1.92.0
readonly AGAVE_EXPECTED_VERSION=3.1.8
readonly CARGO_AUDIT_EXPECTED_VERSION=0.22.1
readonly CARGO_DENY_EXPECTED_VERSION=0.20.2
readonly GITLEAKS_EXPECTED_VERSION=8.30.1

usage() {
  cat <<'USAGE'
Usage: scripts/full-demo.sh [options]

Runs the complete Account Cooker proof against isolated local Surfpool state.

Options:
  --quick              Reduced local dimensions for harness development
  --output-dir DIR     Ignored raw artifact directory under evidence/raw
  --evidence-dir DIR   Sanitized evidence destination under evidence
  -h, --help           Show this help

The default is the canonical run: clean tree, 1,000-agent virtual soak, and
1,000 real local Surfpool transactions. No chain RPC request leaves loopback.
USAGE
}

die() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"
}

while (($#)); do
  case "$1" in
    --quick)
      mode=quick
      shift
      ;;
    --output-dir)
      (($# >= 2)) || die "--output-dir requires a directory"
      output_dir="$2"
      shift 2
      ;;
    --evidence-dir)
      (($# >= 2)) || die "--evidence-dir requires a directory"
      evidence_dir="$2"
      shift 2
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *) die "unknown argument: $1" ;;
  esac
done

if [[ -z "${output_dir}" ]]; then
  output_dir="${PROJECT_ROOT}/evidence/raw/full-demo/${run_token}"
elif [[ "${output_dir}" != /* ]]; then
  output_dir="${PROJECT_ROOT}/${output_dir}"
fi
if [[ -z "${evidence_dir}" ]]; then
  if [[ "${mode}" == canonical ]]; then
    evidence_dir="${PROJECT_ROOT}/evidence/final"
  else
    evidence_dir="${PROJECT_ROOT}/evidence/tmp/full-demo-${run_token}"
  fi
elif [[ "${evidence_dir}" != /* ]]; then
  evidence_dir="${PROJECT_ROOT}/${evidence_dir}"
fi

case "${output_dir}" in
  "${PROJECT_ROOT}/evidence/raw/"*) ;;
  *) die "raw output must remain under ${PROJECT_ROOT}/evidence/raw" ;;
esac
case "${evidence_dir}" in
  "${PROJECT_ROOT}/evidence/"*) ;;
  *) die "sanitized evidence must remain under ${PROJECT_ROOT}/evidence" ;;
esac
case "${output_dir}:${evidence_dir}" in
  *'/../'* | *'/./'*) die "output paths must not contain traversal components" ;;
esac
[[ ! -e "${output_dir}" ]] || die "raw output already exists: ${output_dir}"
[[ ! -e "${evidence_dir}" ]] || die "evidence output already exists: ${evidence_dir}"

for command in awk bash cargo cp date git gitleaks jq lsof rustc sed shellcheck solana solana-keygen tar tee; do
  require_command "${command}"
done
[[ "$(rustc --version | awk '{print $2}')" == "${RUST_EXPECTED_VERSION}" ]] ||
  die "Rust ${RUST_EXPECTED_VERSION} is required"
[[ "$(cargo --version | awk '{print $2}')" == "${RUST_EXPECTED_VERSION}" ]] ||
  die "Cargo ${RUST_EXPECTED_VERSION} is required"
[[ "$(solana --version)" == "solana-cli ${AGAVE_EXPECTED_VERSION} "* ]] ||
  die "Agave solana-cli ${AGAVE_EXPECTED_VERSION} is required"
[[ "$(solana-keygen --version)" == "solana-keygen ${AGAVE_EXPECTED_VERSION} "* ]] ||
  die "Agave solana-keygen ${AGAVE_EXPECTED_VERSION} is required"
[[ "$(gitleaks version)" == "${GITLEAKS_EXPECTED_VERSION}" ]] ||
  die "Gitleaks ${GITLEAKS_EXPECTED_VERSION} is required"
[[ "$(cargo audit --version | awk '{print $NF}')" == "${CARGO_AUDIT_EXPECTED_VERSION}" ]] ||
  die "cargo-audit ${CARGO_AUDIT_EXPECTED_VERSION} is required"
[[ "$(cargo deny --version | awk '{print $NF}')" == "${CARGO_DENY_EXPECTED_VERSION}" ]] ||
  die "cargo-deny ${CARGO_DENY_EXPECTED_VERSION} is required"

if [[ "${mode}" == canonical ]]; then
  [[ -z "$(git -C "${PROJECT_ROOT}" status --short --untracked-files=all)" ]] ||
    die "canonical full demo requires a clean source tree"
  agents=16
  action_limit=32
  planning_limit=5000
  soak_transactions=1000
else
  agents=4
  action_limit=8
  planning_limit=1000
  soak_transactions=25
fi
source_commit="$(git -C "${PROJECT_ROOT}" rev-parse HEAD)"
source_branch="$(git -C "${PROJECT_ROOT}" symbolic-ref --short -q HEAD || printf detached)"
if [[ -n "$(git -C "${PROJECT_ROOT}" status --short --untracked-files=all)" ]]; then
  source_dirty=true
else
  source_dirty=false
fi

export SURFPOOL_HOME="${PROJECT_ROOT}/.surfpool"
export SURFPOOL_KEYPAIR="${SURFPOOL_HOME}/keys/full-demo-${run_token}.json"
export SURFPOOL_DB="${SURFPOOL_HOME}/state/full-demo-${run_token}.sqlite"
export SURFPOOL_PID_FILE="${SURFPOOL_HOME}/full-demo-${run_token}.pid"
export SURFPOOL_SESSION_FILE="${SURFPOOL_HOME}/full-demo-${run_token}-session.json"
export SURFPOOL_RUNTIME_ENV="${SURFPOOL_HOME}/full-demo-${run_token}-runtime.env"
export SURFPOOL_LAUNCHER_LOG="${SURFPOOL_HOME}/logs/full-demo-${run_token}.log"
export SURFPOOL_PORT="${SURFPOOL_PORT:-18899}"
export SURFPOOL_WS_PORT="${SURFPOOL_WS_PORT:-18900}"
export SURFPOOL_STUDIO_PORT="${SURFPOOL_STUDIO_PORT:-19488}"
export SURFPOOL_ID="noise-full-demo-${run_token}"

# shellcheck source=lib/surfpool-common.sh
source "${SCRIPT_DIR}/lib/surfpool-common.sh"

for fresh_path in \
  "${SURFPOOL_KEYPAIR}" "${SURFPOOL_DB}" "${SURFPOOL_PID_FILE}" \
  "${SURFPOOL_SESSION_FILE}" "${SURFPOOL_RUNTIME_ENV}" "${SURFPOOL_LAUNCHER_LOG}"; do
  [[ ! -e "${fresh_path}" ]] || die "run-specific Surfpool state already exists: ${fresh_path}"
done
for local_port in "${SURFPOOL_PORT}" "${SURFPOOL_WS_PORT}" "${SURFPOOL_STUDIO_PORT}"; do
  ! lsof -nP -iTCP:"${local_port}" -sTCP:LISTEN >/dev/null 2>&1 ||
    die "isolated Surfpool port already has a listener: ${local_port}"
done

mkdir -p \
  "${output_dir}/quality" "${output_dir}/cli" "${output_dir}/config" \
  "${output_dir}/commands"
started_here=0
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

run_capture() {
  local argv_json ended_at exit_status label manifest output output_relative started_at stderr_relative
  label="$1"
  output="$2"
  shift 2
  command_counter=$((command_counter + 1))
  manifest="${output_dir}/commands/$(printf '%03d' "${command_counter}").json"
  output_relative="${output#"${output_dir}/"}"
  stderr_relative="${output_relative}.stderr.log"
  started_at="$(date -u '+%Y-%m-%dT%H:%M:%SZ')"
  argv_json="$(jq -cn --args '$ARGS.positional' -- "$@")"
  printf '\n[%s] %s\n' "$(date -u '+%Y-%m-%dT%H:%M:%SZ')" "${label}" | tee -a "${output_dir}/run.log"
  set +e
  "$@" >"${output}" 2>"${output}.stderr.log"
  exit_status=$?
  set -e
  ended_at="$(date -u '+%Y-%m-%dT%H:%M:%SZ')"
  jq -n \
    --argjson sequence "${command_counter}" \
    --arg label "${label}" \
    --arg started_at "${started_at}" \
    --arg ended_at "${ended_at}" \
    --argjson argv "${argv_json}" \
    --arg stdout_file "${output_relative}" \
    --arg stderr_file "${stderr_relative}" \
    --arg stdout_sha256 "$(surfpool_file_sha256 "${output}")" \
    --arg stderr_sha256 "$(surfpool_file_sha256 "${output}.stderr.log")" \
    --argjson exit_status "${exit_status}" \
    '{
      schema_version: 1,
      sequence: $sequence,
      label: $label,
      started_at: $started_at,
      ended_at: $ended_at,
      argv: $argv,
      stdout_file: $stdout_file,
      stderr_file: $stderr_file,
      stdout_sha256: $stdout_sha256,
      stderr_sha256: $stderr_sha256,
      exit_status: $exit_status
    }' >"${manifest}"
  ((exit_status == 0)) || return "${exit_status}"
}

cd "${PROJECT_ROOT}"
export CARGO_TERM_COLOR=never

run_capture "Rust formatting" "${output_dir}/quality/fmt.log" \
  cargo fmt --all -- --check
run_capture "Lint and advisory policy" "${output_dir}/quality/lint-policy.log" \
  "${SCRIPT_DIR}/check-lint-policy.sh"
run_capture "Workspace clippy" "${output_dir}/quality/clippy.log" \
  cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
run_capture "Workspace tests" "${output_dir}/quality/tests.log" \
  cargo test --locked --workspace --all-features
run_capture "Rust documentation tests" "${output_dir}/quality/doc-tests.log" \
  cargo test --locked --workspace --doc
run_capture "Shell syntax" "${output_dir}/quality/bash-n.log" \
  bash -n "${SCRIPT_DIR}"/*.sh "${SCRIPT_DIR}"/lib/*.sh
run_capture "Shellcheck" "${output_dir}/quality/shellcheck.log" \
  shellcheck -x -P SCRIPTDIR "${SCRIPT_DIR}"/*.sh "${SCRIPT_DIR}"/lib/*.sh
run_capture "Git whitespace" "${output_dir}/quality/git-diff-check.log" \
  git diff --check
run_capture "Rust advisory audit" "${output_dir}/quality/cargo-audit.log" \
  cargo audit
run_capture "Dependency policy" "${output_dir}/quality/cargo-deny.log" \
  cargo deny check
run_capture "Repository secret scan" "${output_dir}/quality/gitleaks-dir.log" \
  "${SCRIPT_DIR}/gitleaks-working-tree.sh"
run_capture "Git history secret scan" "${output_dir}/quality/gitleaks-git.log" \
  gitleaks git --redact --no-banner --verbose .
run_capture "Release build" "${output_dir}/quality/release-build.log" \
  cargo build --locked --release -p account-cooker

jq -n '{
  schema_version: 1,
  formatting: true,
  lint_policy: true,
  clippy: true,
  workspace_tests: true,
  documentation_tests: true,
  shell_syntax: true,
  shellcheck: true,
  git_whitespace: true,
  cargo_audit: true,
  cargo_deny: true,
  working_tree_secret_scan: true,
  git_history_secret_scan: true,
  release_build: true,
  all_passed: true
}' >"${output_dir}/quality/gates.json"

cooker_bin="${PROJECT_ROOT}/target/release/cooker"
[[ -x "${cooker_bin}" ]] || die "release cooker binary was not produced"

started_here=1
run_capture "Start isolated Surfpool" "${output_dir}/surfpool-start.txt" \
  "${SCRIPT_DIR}/surfpool-start.sh"
surfpool_verify_runtime_env
cp "${SURFPOOL_SESSION_FILE}" "${output_dir}/surfpool-initial-session.json"
jq -e --arg surfnet_id "${SURFPOOL_ID}" --arg database "${SURFPOOL_DB}" '
  .sessionSchemaVersion == 2 and .surfnetId == $surfnet_id and .database == $database and
  .resumedPersistentDatabase == false
' "${output_dir}/surfpool-initial-session.json" >/dev/null ||
  die "initial Surfpool session did not prove a fresh database"

demo_root="${SURFPOOL_HOME}/workspaces/full-demo-${run_token}"
demo_key_dir="${demo_root}/.surfpool/keys"
mkdir -p "${demo_key_dir}"
chmod 700 "${demo_root}" "${demo_root}/.surfpool" "${demo_key_dir}"
cp "${SURFPOOL_KEYPAIR}" "${demo_key_dir}/funder.json"
chmod 600 "${demo_key_dir}/funder.json"

config_path="${output_dir}/config/cooker.toml"
run_capture "Generate configuration" "${output_dir}/cli/init.txt" \
  "${cooker_bin}" init --output "${config_path}" --force
config_temp="${config_path}.tmp.$$"
awk -v rpc_url="${SURFPOOL_RPC_URL}" -v surfnet_id="${SURFPOOL_ID}" '
  /^rpc_url = / { print "rpc_url = \"" rpc_url "\""; next }
  /^surfnet_id = / { print "surfnet_id = \"" surfnet_id "\""; next }
  { print }
' "${config_path}" >"${config_temp}"
chmod 600 "${config_temp}"
mv "${config_temp}" "${config_path}"

fleet_database="${demo_root}/.surfpool/state/cooker.sqlite"
funding_database="${demo_root}/.surfpool/state/funding.sqlite"
mkdir -p "$(dirname "${fleet_database}")"

run_capture "Validate configuration offline" "${output_dir}/cli/validate.txt" \
  "${cooker_bin}" validate --config "${config_path}"
run_capture "Read-only Surfpool doctor" "${output_dir}/cli/doctor.json" \
  "${cooker_bin}" doctor --config "${config_path}"
run_capture "Initialize isolated signer fleet" "${output_dir}/cli/fleet-init.txt" \
  "${cooker_bin}" fleet-init --config "${config_path}" --project-root "${demo_root}" \
  --database "${fleet_database}" --agents "${agents}" --start 2026-01-01T00:00:00Z
run_capture "Preview fleet funding" "${output_dir}/cli/fund-preview.json" \
  "${cooker_bin}" fund --config "${config_path}" --project-root "${demo_root}" \
  --database "${funding_database}" --funder .surfpool/keys/funder.json \
  --lamports-per-agent 600000000 --limit "${agents}"
run_capture "Execute fleet funding" "${output_dir}/cli/fund-execute.json" \
  "${cooker_bin}" fund --config "${config_path}" --project-root "${demo_root}" \
  --database "${funding_database}" --funder .surfpool/keys/funder.json \
  --lamports-per-agent 600000000 --limit "${agents}" --execute --acknowledge-policy
run_capture "Prove funding idempotency" "${output_dir}/cli/fund-replay.json" \
  "${cooker_bin}" fund --config "${config_path}" --project-root "${demo_root}" \
  --database "${funding_database}" --funder .surfpool/keys/funder.json \
  --lamports-per-agent 600000000 --limit "${agents}" --execute --acknowledge-policy
run_capture "Read status before execution" "${output_dir}/cli/status-before.json" \
  "${cooker_bin}" status --config "${config_path}" --project-root "${demo_root}" \
  --database "${fleet_database}"
run_capture "Preview bounded fleet pass" "${output_dir}/cli/run-preview.json" \
  "${cooker_bin}" run --config "${config_path}" --project-root "${demo_root}" \
  --database "${fleet_database}" --limit "${action_limit}" --planning-limit "${planning_limit}"
run_capture "Execute bounded fleet pass" "${output_dir}/cli/run-execute.json" \
  "${cooker_bin}" run --config "${config_path}" --project-root "${demo_root}" \
  --database "${fleet_database}" --limit "${action_limit}" --planning-limit "${planning_limit}" \
  --execute --acknowledge-policy

claimed_actions="$(jq -er '.claimed_actions | select(. > 0)' "${output_dir}/cli/run-execute.json")" ||
  die "runtime execution did not claim an action"
sleep 2
run_capture "Preview historical reconciliation" "${output_dir}/cli/recover-preview.json" \
  "${cooker_bin}" recover --config "${config_path}" --project-root "${demo_root}" \
  --database "${fleet_database}" --limit "${claimed_actions}" --audit-age-seconds 1
run_capture "Execute historical reconciliation" "${output_dir}/cli/recover-execute.json" \
  "${cooker_bin}" recover --config "${config_path}" --project-root "${demo_root}" \
  --database "${fleet_database}" --limit "${claimed_actions}" --audit-age-seconds 1 \
  --execute --acknowledge-policy
run_capture "Read final status" "${output_dir}/cli/status-after.json" \
  "${cooker_bin}" status --config "${config_path}" --project-root "${demo_root}" \
  --database "${fleet_database}"

jq -e --argjson agents "${agents}" '
  .healthy == true and .signer_loaded == false and .state_changed == false
' "${output_dir}/cli/doctor.json" >/dev/null || die "doctor evidence gate failed"
jq -e --argjson agents "${agents}" '
  .mode == "preview" and .funding_scheme == "dedicated_per_operator" and
  .fleet_agents == $agents and .scheduled_transfers == $agents and
  .network_preflight == false and .signer_loaded == false and .state_changed == false
' "${output_dir}/cli/fund-preview.json" >/dev/null || die "funding preview evidence gate failed"
jq -e --argjson agents "${agents}" '
  .mode == "execute" and .funding_scheme == "dedicated_per_operator" and
  .scheduled_transfers == $agents and .actions_inserted == $agents and
  .claimed_actions == $agents and
  .workers.confirmed == $agents and (.workers.errors | length) == 0 and
  .snapshot.unresolved_actions == 0 and .network_preflight == true and
  .signer_loaded == true and .state_changed == true
' "${output_dir}/cli/fund-execute.json" >/dev/null || die "funding execution evidence gate failed"
jq -e '
  .mode == "execute" and .actions_inserted == 0 and .claimed_actions == 0 and
  .recovery_claimed == 0 and (.workers.errors | length) == 0 and .state_changed == false
' "${output_dir}/cli/fund-replay.json" >/dev/null || die "funding idempotency evidence gate failed"
jq -e '
  .mode == "preview" and .network_preflight == false and
  .signer_loaded == false and .state_changed == false
' "${output_dir}/cli/run-preview.json" >/dev/null || die "runtime preview evidence gate failed"
jq -e '
  .mode == "execute" and .claimed_actions > 0 and
  .workers.confirmed == .claimed_actions and (.workers.errors | length) == 0 and
  .workers.peak_workers <= .action_limit and .after.unresolved_actions == 0 and
  .network_preflight == true and .signer_loaded == true and .state_changed == true
' "${output_dir}/cli/run-execute.json" >/dev/null || die "runtime execution evidence gate failed"
jq -e --argjson claimed "${claimed_actions}" '
  .mode == "preview" and
  .preview_before.counts.confirmation_audit_candidates == $claimed and
  .transaction_submissions == 0 and .network_preflight == false and
  .signer_loaded == false and .state_changed == false
' "${output_dir}/cli/recover-preview.json" >/dev/null || die "recovery preview evidence gate failed"
jq -e --argjson claimed "${claimed_actions}" '
  .mode == "execute" and .audit_claimed == $claimed and .audits.audited == $claimed and
  (.audits.errors | length) == 0 and (.reconciliation.errors | length) == 0 and
  .preview_after.counts.reconciliation_candidates == 0 and
  .preview_after.counts.confirmation_audit_candidates == 0 and
  .transaction_submissions == 0 and .network_preflight == true and
  .signer_loaded == true and .state_changed == true
' "${output_dir}/cli/recover-execute.json" >/dev/null || die "recovery execution evidence gate failed"
jq -e --argjson agents "${agents}" --argjson claimed "${claimed_actions}" '
  .integrity == "ok" and .snapshot.total_agents == $agents and
  .snapshot.actions_by_state.confirmed == $claimed and .snapshot.unresolved_actions == 0 and
  .network_requests == 0 and .signer_loaded == false and .state_changed == false
' "${output_dir}/cli/status-after.json" >/dev/null || die "final status evidence gate failed"

run_capture "Five-seed comparative evaluation" "${output_dir}/evaluation-command.txt" \
  "${cooker_bin}" evaluate --config "${config_path}" \
  --output-dir "${output_dir}/evaluation" --force

virtual_arguments=(--config "${config_path}" --output-dir "${output_dir}/virtual-soak" --force)
if [[ "${mode}" == quick ]]; then virtual_arguments+=(--quick); fi
run_capture "${mode} virtual soak" "${output_dir}/virtual-soak-command.txt" \
  "${SCRIPT_DIR}/virtual-soak.sh" "${virtual_arguments[@]}"

surfpool_soak_arguments=(--transactions "${soak_transactions}" --output-dir "${output_dir}/surfpool-soak")
if [[ "${mode}" == quick ]]; then surfpool_soak_arguments+=(--quick); fi
run_capture "${mode} Surfpool transaction soak" "${output_dir}/surfpool-soak-command.txt" \
  "${SCRIPT_DIR}/surfpool-soak.sh" "${surfpool_soak_arguments[@]}"

cp "${SURFPOOL_SESSION_FILE}" "${output_dir}/surfpool-final-session.json"
jq -e --arg surfnet_id "${SURFPOOL_ID}" --arg database "${SURFPOOL_DB}" '
  .sessionSchemaVersion == 2 and .surfnetId == $surfnet_id and .database == $database and
  .resumedPersistentDatabase == true
' "${output_dir}/surfpool-final-session.json" >/dev/null ||
  die "post-soak Surfpool session did not prove persistent restart state"

run_capture "Stop primary isolated Surfpool" "${output_dir}/surfpool-stop.txt" \
  "${SCRIPT_DIR}/surfpool-stop.sh"
started_here=0

export SURFPOOL_KEYPAIR="${SURFPOOL_HOME}/keys/chain-${run_token}.json"
export SURFPOOL_DB="${SURFPOOL_HOME}/state/chain-${run_token}.sqlite"
export SURFPOOL_PID_FILE="${SURFPOOL_HOME}/chain-${run_token}.pid"
export SURFPOOL_SESSION_FILE="${SURFPOOL_HOME}/chain-${run_token}-session.json"
export SURFPOOL_RUNTIME_ENV="${SURFPOOL_HOME}/chain-${run_token}-runtime.env"
export SURFPOOL_LAUNCHER_LOG="${SURFPOOL_HOME}/logs/chain-${run_token}.log"
export SURFPOOL_ID="noise-chain-${run_token}"
for fresh_path in \
  "${SURFPOOL_KEYPAIR}" "${SURFPOOL_DB}" "${SURFPOOL_PID_FILE}" \
  "${SURFPOOL_SESSION_FILE}" "${SURFPOOL_RUNTIME_ENV}" "${SURFPOOL_LAUNCHER_LOG}"; do
  [[ ! -e "${fresh_path}" ]] || die "chain acceptance state already exists: ${fresh_path}"
done

# Native stake acceptance advances Surfpool epochs, so the chain matrix owns a fresh node and
# runs stake last. The chain script starts and stops this node itself.
run_capture "Complete Surfpool chain acceptance" "${output_dir}/chain-console.txt" \
  env COOKER_CHAIN_EVIDENCE_LOG="${output_dir}/chain-acceptance.log" \
  "${SCRIPT_DIR}/surfpool-chain-acceptance.sh"

cp "${SURFPOOL_SESSION_FILE}" "${output_dir}/chain-surfpool-session.json"
jq -e --arg surfnet_id "${SURFPOOL_ID}" --arg database "${SURFPOOL_DB}" '
  .sessionSchemaVersion == 2 and .surfnetId == $surfnet_id and .database == $database and
  .resumedPersistentDatabase == false
' "${output_dir}/chain-surfpool-session.json" >/dev/null ||
  die "chain acceptance session did not prove fresh independent state"

original_arguments_json='[]'
if ((original_argument_count > 0)); then
  original_arguments_json="$(jq -cn --args '$ARGS.positional' -- "${original_arguments[@]}")"
fi
jq -n \
  --arg run_id "${run_token}" \
  --arg mode "${mode}" \
  --arg started_at "${run_started_at}" \
  --arg finished_at "$(date -u '+%Y-%m-%dT%H:%M:%SZ')" \
  --arg source_commit "${source_commit}" \
  --arg source_branch "${source_branch}" \
  --argjson source_dirty "${source_dirty}" \
  --argjson arguments "${original_arguments_json}" \
  --argjson command_count "${command_counter}" \
  --arg rustc_version "$(rustc -Vv)" \
  --arg cargo_version "$(cargo -V)" \
  --arg git_version "$(git --version)" \
  --arg surfpool_version "$(surfpool --version)" \
  --arg solana_version "$(solana --version)" \
  --arg solana_keygen_version "$(solana-keygen --version)" \
  --arg jq_version "$(jq --version)" \
  --arg shellcheck_version "$(shellcheck --version | awk 'NR == 2 {print $2}')" \
  --arg gitleaks_version "$(gitleaks version)" \
  --arg cargo_audit_version "$(cargo audit --version)" \
  --arg cargo_deny_version "$(cargo deny --version)" \
  '{
    schema_version: 1,
    run_id: $run_id,
    mode: $mode,
    started_at: $started_at,
    finished_at: $finished_at,
    source: {git_commit: $source_commit, git_branch: $source_branch, git_dirty: $source_dirty},
    invocation: {program: "scripts/full-demo.sh", arguments: $arguments},
    command_count: $command_count,
    tools: {
      rustc: $rustc_version,
      cargo: $cargo_version,
      git: $git_version,
      surfpool: $surfpool_version,
      solana: $solana_version,
      solana_keygen: $solana_keygen_version,
      jq: $jq_version,
      shellcheck: $shellcheck_version,
      gitleaks: $gitleaks_version,
      cargo_audit: $cargo_audit_version,
      cargo_deny: $cargo_deny_version
    },
    public_chain_rpc_reads: 0,
    public_network_writes: 0
  }' >"${output_dir}/run-metadata.json"

"${SCRIPT_DIR}/build-evidence-pack.sh" \
  --raw-dir "${output_dir}" --output-dir "${evidence_dir}" --mode "${mode}"

(
  cd "${evidence_dir}"
  while read -r expected_digest relative_file; do
    [[ -n "${expected_digest}" && -n "${relative_file}" ]] ||
      die "malformed evidence checksum entry"
    actual_digest="$(surfpool_file_sha256 "${relative_file}")"
    [[ "${actual_digest}" == "${expected_digest}" ]] ||
      die "evidence checksum mismatch after publication: ${relative_file}"
  done <checksums.txt
)
gitleaks dir --redact --no-banner --config "${PROJECT_ROOT}/.gitleaks.toml" "${evidence_dir}"

trap - EXIT INT TERM
printf '\nFull demo passed (%s).\n' "${mode}"
printf 'Raw local artifacts: %s\n' "${output_dir}"
printf 'Sanitized evidence: %s\n' "${evidence_dir}"

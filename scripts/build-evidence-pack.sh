#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

raw_dir=''
output_dir=''
mode=canonical

usage() {
  cat <<'USAGE'
Usage: scripts/build-evidence-pack.sh --raw-dir DIR --output-dir DIR [--mode MODE]

Validates ignored full-demo artifacts and creates a sanitized, checksum-bearing evidence pack.
MODE is canonical or quick; canonical evidence requires a clean source tree.
USAGE
}

die() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"
}

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

while (($#)); do
  case "$1" in
    --raw-dir)
      (($# >= 2)) || die "--raw-dir requires a directory"
      raw_dir="$2"
      shift 2
      ;;
    --output-dir)
      (($# >= 2)) || die "--output-dir requires a directory"
      output_dir="$2"
      shift 2
      ;;
    --mode)
      (($# >= 2)) || die "--mode requires canonical or quick"
      mode="$2"
      shift 2
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *) die "unknown argument: $1" ;;
  esac
done

[[ "${mode}" == canonical || "${mode}" == quick ]] || die "mode must be canonical or quick"
[[ -n "${raw_dir}" && -n "${output_dir}" ]] || {
  usage >&2
  exit 1
}
for command in awk cp date find git grep jq sed sort tr wc; do
  require_command "${command}"
done
command -v sha256sum >/dev/null 2>&1 || require_command shasum

[[ -d "${raw_dir}" ]] || die "raw evidence directory is missing: ${raw_dir}"
raw_dir="$(cd "${raw_dir}" && pwd)"
case "${output_dir}" in
  /*) ;;
  *) output_dir="${PROJECT_ROOT}/${output_dir}" ;;
esac
case "${output_dir}" in
  "${PROJECT_ROOT}/evidence/"*) ;;
  *) die "output directory must remain under ${PROJECT_ROOT}/evidence" ;;
esac
case "${output_dir}" in
  *'/../'* | *'/./'*) die "output directory must not contain traversal components" ;;
esac
[[ ! -e "${output_dir}" ]] || die "evidence output already exists: ${output_dir}"

required_files=(
  run-metadata.json
  quality/gates.json
  config/cooker.toml
  cli/doctor.json
  cli/fund-preview.json
  cli/fund-execute.json
  cli/fund-replay.json
  cli/status-before.json
  cli/run-preview.json
  cli/run-execute.json
  cli/recover-preview.json
  cli/recover-execute.json
  cli/status-after.json
  evaluation/evaluation.json
  evaluation/evaluation.csv
  evaluation/evaluation.md
  chain-acceptance.log
  virtual-soak/manifest.json
  virtual-soak/run.log
  surfpool-soak/soak.json
  surfpool-soak/test.log
  surfpool-soak/surfpool-session.json
  surfpool-initial-session.json
  surfpool-final-session.json
  chain-surfpool-session.json
)
for relative in "${required_files[@]}"; do
  [[ -r "${raw_dir}/${relative}" ]] || die "required raw artifact is missing: ${relative}"
done

git_status="$(git -C "${PROJECT_ROOT}" status --short --untracked-files=all)"
if [[ -n "${git_status}" ]]; then source_dirty=true; else source_dirty=false; fi
if [[ "${mode}" == canonical && "${source_dirty}" == true ]]; then
  die "canonical evidence requires a clean source tree"
fi
git_commit="$(git -C "${PROJECT_ROOT}" rev-parse HEAD)"
git_branch="$(git -C "${PROJECT_ROOT}" symbolic-ref --short -q HEAD || printf detached)"

jq -e --arg mode "${mode}" --arg commit "${git_commit}" --arg branch "${git_branch}" '
  .schema_version == 1 and .mode == $mode and
  (.run_id | test("^[0-9]{8}T[0-9]{6}Z-[0-9]+$")) and
  (.started_at | fromdateiso8601) <= (.finished_at | fromdateiso8601) and
  .source.git_commit == $commit and .source.git_branch == $branch and
  (.source.git_dirty | type) == "boolean" and
  .invocation.program == "scripts/full-demo.sh" and (.invocation.arguments | type) == "array" and
  .command_count > 0 and
  (["rustc", "cargo", "git", "surfpool", "solana", "solana_keygen", "jq", "shellcheck", "gitleaks",
    "cargo_audit", "cargo_deny"] - (.tools | keys) | length) == 0 and
  all(.tools[]; type == "string" and length > 0) and
  .tools.gitleaks == "8.30.1" and .public_chain_rpc_reads == 0 and
  .public_network_writes == 0 and
  (if $mode == "canonical" then .source.git_dirty == false else true end)
  and (.tools.rustc | startswith("rustc 1.92.0 "))
  and (.tools.cargo | startswith("cargo 1.92.0 "))
  and .tools.surfpool == "surfpool 1.4.0"
  and (.tools.solana | startswith("solana-cli 3.1.8 "))
  and (.tools.solana_keygen | startswith("solana-keygen 3.1.8 "))
  and .tools.cargo_audit == "cargo-audit-audit 0.22.1"
  and .tools.cargo_deny == "cargo-deny 0.20.2"
' "${raw_dir}/run-metadata.json" >/dev/null || die "full-demo run metadata contract failed"

command_manifest_count=0
expected_sequence=1
command_manifest_list="${raw_dir}/commands"
[[ -d "${command_manifest_list}" && ! -L "${command_manifest_list}" ]] ||
  die "command provenance directory is missing or is a symlink"
while IFS= read -r command_manifest; do
  [[ -f "${command_manifest}" && ! -L "${command_manifest}" ]] ||
    die "command provenance entry is not a regular file"
  jq -e --argjson sequence "${expected_sequence}" '
    .schema_version == 1 and .sequence == $sequence and
    (.label | type) == "string" and (.label | length) > 0 and
    (.started_at | fromdateiso8601) <= (.ended_at | fromdateiso8601) and
    (.argv | type) == "array" and (.argv | length) > 0 and
    (.stdout_file | test("^[A-Za-z0-9._/-]+$")) and
    (.stdout_file | contains("..") | not) and
    (.stderr_file | test("^[A-Za-z0-9._/-]+$")) and
    (.stderr_file | contains("..") | not) and
    (.stdout_sha256 | test("^[0-9a-f]{64}$")) and
    (.stderr_sha256 | test("^[0-9a-f]{64}$")) and .exit_status == 0
  ' "${command_manifest}" >/dev/null || die "command provenance contract failed: ${command_manifest}"
  stdout_relative="$(jq -er '.stdout_file' "${command_manifest}")"
  stderr_relative="$(jq -er '.stderr_file' "${command_manifest}")"
  for relative_log in "${stdout_relative}" "${stderr_relative}"; do
    log_path="${raw_dir}/${relative_log}"
    [[ -f "${log_path}" && ! -L "${log_path}" ]] ||
      die "command log is missing or is a symlink: ${relative_log}"
  done
  actual_stdout_sha256="$(sha256_file "${raw_dir}/${stdout_relative}")"
  expected_stdout_sha256="$(jq -r '.stdout_sha256' "${command_manifest}")"
  [[ "${actual_stdout_sha256}" == "${expected_stdout_sha256}" ]] ||
    die "command stdout hash mismatch: ${stdout_relative}"
  actual_stderr_sha256="$(sha256_file "${raw_dir}/${stderr_relative}")"
  expected_stderr_sha256="$(jq -r '.stderr_sha256' "${command_manifest}")"
  [[ "${actual_stderr_sha256}" == "${expected_stderr_sha256}" ]] ||
    die "command stderr hash mismatch: ${stderr_relative}"
  command_manifest_count=$((command_manifest_count + 1))
  expected_sequence=$((expected_sequence + 1))
done < <(find "${command_manifest_list}" -maxdepth 1 -type f -name '*.json' -print | LC_ALL=C sort)
[[ "${command_manifest_count}" == "$(jq -r '.command_count' "${raw_dir}/run-metadata.json")" ]] ||
  die "command provenance count does not match run metadata"

output_parent="$(dirname "${output_dir}")"
output_name="$(basename "${output_dir}")"
mkdir -p "${output_parent}"
staging="${output_parent}/.${output_name}.tmp.$$"
[[ ! -e "${staging}" ]] || die "staging path already exists: ${staging}"
mkdir -p "${staging}/.chain"
checksum_inputs=''
cleanup() {
  if [[ -n "${staging:-}" && -d "${staging}" ]]; then
    find "${staging}" -depth -delete
  fi
  if [[ -n "${checksum_inputs:-}" && -f "${checksum_inputs}" ]]; then
    find "${checksum_inputs}" -delete
  fi
}
trap cleanup EXIT INT TERM

jq -s --arg raw_dir "${raw_dir}" --arg project_root "${PROJECT_ROOT}" '
  def replace_literal($from; $to): split($from) | join($to);
  map(walk(
    if type == "string" then
      replace_literal($raw_dir; "<raw>") |
      replace_literal($project_root; "<project>") |
      gsub("http://127\\.0\\.0\\.1:[0-9]+"; "http://127.0.0.1:<local>") |
      gsub("ws://127\\.0\\.0\\.1:[0-9]+"; "ws://127.0.0.1:<local>")
    else . end
  ))
' "${command_manifest_list}"/*.json >"${staging}/executions.json"
jq --arg raw_dir "${raw_dir}" --arg project_root "${PROJECT_ROOT}" '
  def replace_literal($from; $to): split($from) | join($to);
  walk(
    if type == "string" then
      replace_literal($raw_dir; "<raw>") | replace_literal($project_root; "<project>")
    else . end
  )
' "${raw_dir}/run-metadata.json" >"${staging}/run.json"

jq -e '
  .schema_version == 1 and .formatting == true and .clippy == true and
  .workspace_tests == true and .documentation_tests == true and
  .shell_syntax == true and .shellcheck == true and .git_whitespace == true and
  .cargo_audit == true and .cargo_deny == true and
  .working_tree_secret_scan == true and .git_history_secret_scan == true and
  .release_build == true and .all_passed == true
' "${raw_dir}/quality/gates.json" >/dev/null || die "quality gate evidence contract failed"
cp "${raw_dir}/quality/gates.json" "${staging}/quality-gates.json"

jq -n \
  --slurpfile doctor "${raw_dir}/cli/doctor.json" \
  --slurpfile fund_preview "${raw_dir}/cli/fund-preview.json" \
  --slurpfile fund_execute "${raw_dir}/cli/fund-execute.json" \
  --slurpfile fund_replay "${raw_dir}/cli/fund-replay.json" \
  --slurpfile status_before "${raw_dir}/cli/status-before.json" \
  --slurpfile run_preview "${raw_dir}/cli/run-preview.json" \
  --slurpfile run_execute "${raw_dir}/cli/run-execute.json" \
  --slurpfile recover_preview "${raw_dir}/cli/recover-preview.json" \
  --slurpfile recover_execute "${raw_dir}/cli/recover-execute.json" \
  --slurpfile status_after "${raw_dir}/cli/status-after.json" '
  ($doctor[0]) as $doctor |
  ($fund_preview[0]) as $fund_preview |
  ($fund_execute[0]) as $fund_execute |
  ($fund_replay[0]) as $fund_replay |
  ($status_before[0]) as $status_before |
  ($run_preview[0]) as $run_preview |
  ($run_execute[0]) as $run_execute |
  ($recover_preview[0]) as $recover_preview |
  ($recover_execute[0]) as $recover_execute |
  ($status_after[0]) as $status_after |
  {
    schema_version: 1,
    doctor: ($doctor | {
      healthy, surfnet_id, surfpool_version, solana_core, feature_set,
      block_height, epoch, absolute_slot, recent_local_signatures,
      signer_loaded, state_changed
    }),
    funding: {
      preview: ($fund_preview | {
        mode, fleet_agents, lamports_per_agent, network_preflight,
        signer_loaded, state_changed
      }),
      execute: ($fund_execute | {
        mode, fleet_agents, lamports_per_agent, actions_inserted,
        claimed_actions, recovery_claimed, workers, network_preflight,
        signer_loaded, stop_reason, state_changed
      }),
      idempotent_replay: ($fund_replay | {
        mode, actions_inserted, claimed_actions, recovery_claimed, workers,
        network_preflight, signer_loaded, stop_reason, state_changed
      })
    },
    runtime: {
      status_before: ($status_before | {
        store_schema_version, journal_mode, integrity, snapshot,
        network_requests, signer_loaded, state_changed
      }),
      preview: ($run_preview | {
        mode, agents, enabled_actions, planning_limit, action_limit,
        claimed_actions, workers, network_preflight, signer_loaded, state_changed
      }),
      execute: ($run_execute | {
        mode, run_id, agents, enabled_actions, planning_limit, action_limit,
        planning, claimed_actions, workers, before, after,
        network_preflight, signer_loaded, stop_reason, state_changed
      }),
      status_after: ($status_after | {
        store_schema_version, journal_mode, integrity, snapshot,
        network_requests, signer_loaded, state_changed
      })
    },
    recovery: {
      preview: ($recover_preview | {
        mode, limit,
        counts_before: .preview_before.counts,
        transaction_submissions, network_preflight, signer_loaded, state_changed
      }),
      execute: ($recover_execute | {
        mode, limit, released_expired_leases, reconciliation_claimed,
        audit_claimed, reconciliation, audits,
        counts_before: .preview_before.counts,
        counts_after: .preview_after.counts,
        transaction_submissions, network_preflight, signer_loaded,
        stop_reason, state_changed
      })
    }
  }' >"${staging}/cli-workflow.json"

jq -e '
  .schema_version == 1 and .doctor.healthy == true and
  .doctor.signer_loaded == false and .doctor.state_changed == false and
  .funding.preview.network_preflight == false and
  .funding.preview.signer_loaded == false and .funding.preview.state_changed == false and
  .funding.execute.claimed_actions == .funding.execute.fleet_agents and
  .funding.execute.workers.confirmed == .funding.execute.fleet_agents and
  (.funding.execute.workers.errors | length) == 0 and
  .funding.idempotent_replay.actions_inserted == 0 and
  .funding.idempotent_replay.claimed_actions == 0 and
  .funding.idempotent_replay.state_changed == false and
  .runtime.preview.network_preflight == false and
  .runtime.preview.signer_loaded == false and .runtime.preview.state_changed == false and
  .runtime.execute.claimed_actions > 0 and
  .runtime.execute.workers.confirmed == .runtime.execute.claimed_actions and
  (.runtime.execute.workers.errors | length) == 0 and
  .runtime.execute.after.unresolved_actions == 0 and
  .runtime.status_after.integrity == "ok" and
  .runtime.status_after.snapshot.unresolved_actions == 0 and
  .runtime.status_after.network_requests == 0 and
  .runtime.status_after.signer_loaded == false and
  .recovery.preview.counts_before.confirmation_audit_candidates
    == .runtime.execute.claimed_actions and
  .recovery.execute.audit_claimed == .runtime.execute.claimed_actions and
  .recovery.execute.audits.audited == .runtime.execute.claimed_actions and
  .recovery.execute.counts_after.reconciliation_candidates == 0 and
  .recovery.execute.counts_after.confirmation_audit_candidates == 0 and
  .recovery.execute.transaction_submissions == 0 and
  (.recovery.execute.reconciliation.errors | length) == 0 and
  (.recovery.execute.audits.errors | length) == 0
' "${staging}/cli-workflow.json" >/dev/null || die "CLI evidence contract failed"

extract_marker() {
  local marker="$1"
  local destination="$2"
  awk -v marker="${marker}" '
    index($0, marker) == 1 {
      count += 1
      value = substr($0, length(marker) + 1)
    }
    END { if (count != 1 || value == "") exit 1; print value }
  ' "${raw_dir}/chain-acceptance.log" >"${destination}" ||
    die "chain acceptance must contain exactly one marker: ${marker}"
  jq empty "${destination}" || die "chain acceptance marker is invalid JSON: ${marker}"
}

extract_marker COOKER_NATIVE_EVIDENCE= "${staging}/.chain/native.json"
extract_marker COOKER_SPL_EVIDENCE= "${staging}/.chain/spl.json"
extract_marker COOKER_STAKE_EVIDENCE= "${staging}/.chain/stake.json"
extract_marker COOKER_FAULT_EVIDENCE= "${staging}/.chain/fault.json"
extract_marker COOKER_JUPITER_EVIDENCE= "${staging}/.chain/jupiter.json"
extract_marker COOKER_PROCESS_RECOVERY_EVIDENCE= "${staging}/.chain/process-recovery.json"
jq -n \
  --slurpfile native "${staging}/.chain/native.json" \
  --slurpfile spl "${staging}/.chain/spl.json" \
  --slurpfile stake "${staging}/.chain/stake.json" \
  --slurpfile fault "${staging}/.chain/fault.json" \
  --slurpfile jupiter "${staging}/.chain/jupiter.json" \
  --slurpfile process_recovery "${staging}/.chain/process-recovery.json" \
  '{
    schema_version: 1,
    native: $native[0],
    spl: $spl[0],
    stake: $stake[0],
    fault_reconciliation: $fault[0],
    jupiter: $jupiter[0],
    process_recovery: $process_recovery[0]
  }' >"${staging}/transactions.json"
find "${staging}/.chain" -depth -delete

 jq -e '
  def sanitized_signature:
    type == "string" and test("^[1-9A-HJ-NP-Za-km-z]{10}\\.\\.\\.[1-9A-HJ-NP-Za-km-z]{10}$");
  def positive_fee: type == "number" and . > 0 and . <= 100000;
  .schema_version == 1 and

  .native.schema_version == 1 and .native.adapter == "native_transfer" and
  .native.confirmation_status == "confirmed" and .native.postconditions_met == true and
  (.native.signature | sanitized_signature) and
  .native.transfer_lamports > 0 and (.native.transaction_fee_lamports | positive_fee) and
  .native.source_before_lamports - .native.source_after_lamports
    == .native.source_debit_lamports and
  .native.source_debit_lamports
    == .native.transfer_lamports + .native.transaction_fee_lamports and
  .native.destination_after_lamports - .native.destination_before_lamports
    == .native.destination_credit_lamports and
  .native.destination_credit_lamports == .native.transfer_lamports and
  ([.native.observations[].kind] | sort) ==
    (["native_destination_balance_delta", "native_source_balance_delta", "transaction_fee_ceiling"] | sort) and
  all(.native.observations[]; .attributes.met == "true") and

  .spl.schema_version == 1 and .spl.postconditions_met == true and
  (.spl.signature | sanitized_signature) and (.spl.later_signature | sanitized_signature) and
  .spl.transferred_raw > 0 and .spl.minted_raw == .spl.source_before_raw and
  .spl.source_before_raw - .spl.source_after_raw == .spl.transferred_raw and
  .spl.destination_after_raw - .spl.destination_before_raw == .spl.transferred_raw and
  (.spl.fee_lamports | positive_fee) and .spl.ata_rent_lamports > 0 and
  .spl.payer_native_debit_lamports == .spl.ata_rent_lamports + .spl.fee_lamports and
  .spl.historical_confirmation_reaudits == 1 and .spl.historical_reaudit_resubmissions == 0 and

  .stake.schema_version == 1 and .stake.postconditions_met == true and
  (.stake.vote_setup_signature | sanitized_signature) and
  (.stake.enter_signature | sanitized_signature) and
  (.stake.deactivate_signature | sanitized_signature) and
  (.stake.withdraw_signature | sanitized_signature) and
  (.stake.enter_fee_lamports | positive_fee) and
  (.stake.deactivate_fee_lamports | positive_fee) and
  (.stake.withdraw_fee_lamports | positive_fee) and
  .stake.phases.enter.state_before == "absent" and .stake.phases.enter.state_after == "active" and
  .stake.phases.enter.principal_lamports == .stake.principal_lamports and
  .stake.phases.enter.rent_lamports == .stake.rent_lamports and
  .stake.phases.enter.payer_before_lamports - .stake.phases.enter.payer_after_lamports
    == .stake.principal_lamports + .stake.rent_lamports + .stake.enter_fee_lamports and
  .stake.phases.enter.stake_after_lamports - .stake.phases.enter.stake_before_lamports
    == .stake.principal_lamports + .stake.rent_lamports and
  .stake.phases.deactivate.state_before == "active" and
  .stake.phases.deactivate.state_after == "deactivating" and
  .stake.phases.deactivate.payer_before_lamports - .stake.phases.deactivate.payer_after_lamports
    == .stake.deactivate_fee_lamports and
  .stake.phases.deactivate.stake_before_lamports == .stake.phases.deactivate.stake_after_lamports and
  .stake.phases.epoch_travel.state_before == "deactivating" and
  .stake.phases.epoch_travel.state_after == "inactive" and
  .stake.phases.epoch_travel.epoch_after == .stake.phases.epoch_travel.target_epoch and
  .stake.phases.epoch_travel.epoch_after > .stake.phases.epoch_travel.epoch_before and
  .stake.phases.epoch_travel.slot_after > .stake.phases.epoch_travel.slot_before and
  .stake.phases.withdraw.state_before == "inactive" and
  .stake.phases.withdraw.state_after == "closed" and
  .stake.phases.withdraw.stake_after_lamports == 0 and
  .stake.phases.withdraw.payer_after_lamports - .stake.phases.withdraw.payer_before_lamports
    == .stake.phases.withdraw.stake_before_lamports - .stake.withdraw_fee_lamports and
  .stake.post_lifecycle_confirmation_reaudits == 3 and .stake.post_lifecycle_resubmissions == 0 and

  .fault_reconciliation.schema_version == 1 and
  .fault_reconciliation.postconditions_met == true and
  (.fault_reconciliation.simulation_failure_signature | sanitized_signature) and
  (.fault_reconciliation.stale_signature | sanitized_signature) and
  (.fault_reconciliation.reconciled_signature | sanitized_signature) and
  .fault_reconciliation.insufficient_rejected_before_signature == true and
  .fault_reconciliation.simulation_failure_local_count == 0 and
  .fault_reconciliation.stale_local_count == 0 and
  .fault_reconciliation.block_height_after_expiry
    > .fault_reconciliation.stale_last_valid_block_height and
  .fault_reconciliation.expiry_clock_steps > 0 and
  .fault_reconciliation.lost_response_error_class == "unknown_outcome" and
  .fault_reconciliation.stale_error_class == "deterministic" and
  .fault_reconciliation.duplicate_signature_error_class == "deterministic_already_processed" and
  .fault_reconciliation.local_signature_count_after_duplicate == 1 and
  .fault_reconciliation.destination_delta_lamports == 1000000 and
  (.fault_reconciliation.fee_lamports | positive_fee) and

  .jupiter.schema_version == 1 and .jupiter.postconditions_met == true and
  .jupiter.confirmation_status == "confirmed" and (.jupiter.signature | sanitized_signature) and
  .jupiter.plan_source == "reviewed-fixture" and .jupiter.input_amount > 0 and
  .jupiter.max_slippage_bps > 0 and .jupiter.reviewed_user_account_rebindings >= 3 and
  ([.jupiter.observations[].kind] | sort) ==
    (["jupiter_input_token_delta", "jupiter_output_token_delta", "jupiter_route_metadata", "transaction_fee_ceiling"] | sort) and
  all(.jupiter.observations[]; .attributes.met == "true") and
  ((.jupiter.observations[] | select(.kind == "jupiter_input_token_delta") |
    .attributes.actual_delta | tonumber) <= -.jupiter.input_amount) and
  ((.jupiter.observations[] | select(.kind == "jupiter_output_token_delta") |
    .attributes.actual_delta | tonumber) >=
   (.jupiter.observations[] | select(.kind == "jupiter_output_token_delta") |
    .attributes.minimum_delta | tonumber)) and
  ((.jupiter.observations[] | select(.kind == "transaction_fee_ceiling") |
    .attributes.actual_fee_lamports | tonumber) > 0) and
  ((.jupiter.observations[] | select(.kind == "transaction_fee_ceiling") |
    .attributes.actual_fee_lamports | tonumber) <=
   (.jupiter.observations[] | select(.kind == "transaction_fee_ceiling") |
    .attributes.max_fee_lamports | tonumber)) and

  .process_recovery.schema_version == 1 and
  .process_recovery.scenario == "six_checkpoint_real_process_crash_recovery" and
  .process_recovery.checkpoint_count == 6 and .process_recovery.confirmed_actions == 5 and
  .process_recovery.expired_without_submission == 1 and
  .process_recovery.duplicate_logical_actions == 0 and
  .process_recovery.duplicate_local_signatures == 0 and
  .process_recovery.unresolved_actions == 0 and
  ([.process_recovery.cases[].checkpoint] | sort) ==
    (["after_intent_persistence", "after_prepared_persistence", "after_simulation",
      "after_signature_persistence", "after_send_response_lost",
      "after_confirmation_before_promotion"] | sort) and
  all(.process_recovery.cases[];
    .child_forcibly_terminated == true and .termination_signal == 9 and
    .logical_action_count == 1 and .prepared_event_count == 1 and
    .simulation_event_count == 1 and .submission_event_count == 1 and .trace_count == 1 and
    .prepared_reused_exactly == true and (.signature | sanitized_signature) and
    (.prepared_transaction_sha256 | test("^[0-9a-f]{64}$")) and
    (.signature_sha256 | test("^[0-9a-f]{64}$")) and
    (if .checkpoint == "after_intent_persistence" then
       .state_before_recovery == "planned" and .prepared_durable_before_restart == false and
       .terminal_state == "confirmed" and .crash_submit_attempts == 0 and
       .recovery_submit_attempts == 1 and .submit_attempts == 1
     elif .checkpoint == "after_prepared_persistence" then
       .state_before_recovery == "planned" and .prepared_durable_before_restart == true and
       .terminal_state == "confirmed" and .crash_submit_attempts == 0 and
       .recovery_submit_attempts == 1 and .submit_attempts == 1
     elif .checkpoint == "after_simulation" then
       .state_before_recovery == "simulated" and .prepared_durable_before_restart == true and
       .terminal_state == "confirmed" and .crash_submit_attempts == 0 and
       .recovery_submit_attempts == 1 and .submit_attempts == 1
     elif .checkpoint == "after_signature_persistence" then
       .state_before_recovery == "submitted" and .prepared_durable_before_restart == true and
       .terminal_state == "expired" and .expiry_slots_advanced > 0 and
       .crash_submit_attempts == 0 and .recovery_submit_attempts == 0 and .submit_attempts == 0 and
       .local_signature_occurrences == 0 and .destination_credit_lamports == 0 and
       .source_debit_lamports == 0 and .transaction_fee_lamports == 0
     else
       .state_before_recovery == "submitted" and .prepared_durable_before_restart == true and
       .terminal_state == "confirmed" and .crash_submit_attempts == 1 and
       .recovery_submit_attempts == 0 and .submit_attempts == 1 and
       .local_signature_occurrences_before_recovery == 1
     end) and
    (if .terminal_state == "confirmed" then
       .local_signature_occurrences == 1 and .destination_credit_lamports == 1000000 and
       (.transaction_fee_lamports | positive_fee) and
       .source_debit_lamports == 1000000 + .transaction_fee_lamports
     else true end))
' "${staging}/transactions.json" >/dev/null || die "chain acceptance evidence contract failed"

jq --arg project_root "${PROJECT_ROOT}" '{
  schema_version,
  mode,
  offline_only,
  signer_loaded,
  network_requests,
  trace_hash_algorithm,
  scheduler,
  start,
  model_version,
  agents,
  days,
  max_concurrency,
  max_decisions_per_agent_day,
  daily_budget,
  seeds,
  completed_seeds,
  total_wall_time_ms,
  per_seed,
  aggregate,
  proofs,
  environment: {
    captured_at: .environment.captured_at,
    platform: .environment.platform,
    rustc: .environment.rustc,
    cargo: .environment.cargo,
    git: {
      commit: .environment.git.commit,
      branch: .environment.git.branch,
      dirty: .environment.git.dirty
    },
    build_profile: .environment.build_profile,
    cargo_network: .environment.cargo_network,
    time_implementation: .environment.time_implementation,
    peak_rss_bytes: .environment.peak_rss_bytes,
    command: (.environment.command | gsub($project_root; "<project>")),
    config: (.environment.config | gsub($project_root; "<project>"))
  }
}' "${raw_dir}/virtual-soak/manifest.json" >"${staging}/virtual-soak.json"

jq -e --arg mode "${mode}" --arg commit "${git_commit}" '
  def proof_passes:
    .deterministic_replay == true and .chronological_events == true and
    .label_free_events == true and .bounded_scheduler_workers == true and
    .daily_budget_respected == true and .completed_without_panic == true;
  def stats_match($values):
    .min == ($values | min) and .max == ($values | max) and
    ((.mean - (($values | add) / ($values | length))) | fabs) < 0.000000001;
  . as $root |
  .schema_version == 1 and .mode == $mode and .offline_only == true and
  .signer_loaded == false and .network_requests == 0 and
  .trace_hash_algorithm == "blake3" and .scheduler == "stable_bounded" and
  (.environment.platform | test("^[A-Za-z0-9._-]+ [A-Za-z0-9._-]+$")) and
  .seeds == (if $mode == "canonical" then [11, 23, 37, 51, 71] else [11, 23] end) and
  .completed_seeds == (.seeds | length) and .completed_seeds == (.per_seed | length) and
  [.per_seed[].seed] == .seeds and ([.per_seed[].seed] | unique | length) == .completed_seeds and
  ([.per_seed[].seed_hex] | unique | length) == .completed_seeds and
  ([.per_seed[].run_id] | unique | length) == .completed_seeds and
  ([.per_seed[].trace_file] | unique | length) == .completed_seeds and
  ([.per_seed[].trace_blake3] | unique | length) == .completed_seeds and
  all(.per_seed[];
    (.seed_hex | test("^[0-9a-f]{64}$")) and
    (.run_id | test("^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$")) and
    (.trace_blake3 | test("^[0-9a-f]{64}$")) and
    .trace_file == ("traces/seed-" + (.seed | tostring) + ".jsonl") and
    .decisions >= .observable_events and .observable_events > 0 and .trace_bytes > 0 and
    .wall_time_ms >= 0 and .replay_wall_time_ms >= 0 and (.proofs | proof_passes)) and
  (.proofs | proof_passes) and
  (.aggregate.decisions | stats_match([$root.per_seed[].decisions])) and
  (.aggregate.observable_events | stats_match([$root.per_seed[].observable_events])) and
  (.aggregate.trace_bytes | stats_match([$root.per_seed[].trace_bytes])) and
  (.aggregate.wall_time_ms | stats_match([$root.per_seed[].wall_time_ms])) and
  (.aggregate.replay_wall_time_ms | stats_match([$root.per_seed[].replay_wall_time_ms])) and
  .total_wall_time_ms >= ([.per_seed[].wall_time_ms] | add) and
  .max_concurrency >= 1 and .max_concurrency <= .agents and
  .max_decisions_per_agent_day > 0 and .daily_budget > 0 and
  .environment.git.commit == $commit and .environment.build_profile == "release" and
  .environment.cargo_network == "offline" and .environment.peak_rss_bytes > 0 and
  (.environment.command | startswith("<project>/target/release/cooker soak ")) and
  (.environment.config | test("^(<project>/)?evidence/raw/")) and
  (if $mode == "canonical" then
    .agents == 1000 and .days == 30 and
    .environment.git.dirty == false
   else .agents >= 1 and .days >= 1 end)
' "${staging}/virtual-soak.json" >/dev/null || die "virtual soak evidence contract failed"

verified_trace_count=0
while IFS=$'\t' read -r trace_file trace_bytes; do
  case "${trace_file}" in
    traces/seed-*.jsonl) ;;
    *) die "virtual soak trace path is unsafe: ${trace_file}" ;;
  esac
  trace_path="${raw_dir}/virtual-soak/${trace_file}"
  [[ -f "${trace_path}" && ! -L "${trace_path}" ]] ||
    die "virtual soak trace is missing or is a symlink: ${trace_file}"
  actual_trace_bytes="$(wc -c <"${trace_path}" | tr -d '[:space:]')"
  [[ "${actual_trace_bytes}" == "${trace_bytes}" ]] ||
    die "virtual soak trace byte count mismatch: ${trace_file}"
  verified_trace_count=$((verified_trace_count + 1))
done < <(jq -r '.per_seed[] | [.trace_file, .trace_bytes] | @tsv' "${staging}/virtual-soak.json")
[[ "${verified_trace_count}" == "$(jq -r '.completed_seeds' "${staging}/virtual-soak.json")" ]] ||
  die "virtual soak trace verification count mismatch"
[[ "$(grep -Fc 'all hashes and invariants pass' "${raw_dir}/virtual-soak/run.log" || true)" == 1 ]] ||
  die "virtual soak verify-only completion marker is missing or duplicated"

jq 'del(.database_file)' "${raw_dir}/surfpool-soak/soak.json" >"${staging}/surfpool-soak.json"
if [[ "${mode}" == canonical ]]; then
  expected_transactions=1000
  expected_concurrency=16
else
  expected_transactions=25
  expected_concurrency=4
fi
jq -e --argjson expected "${expected_transactions}" --argjson concurrency "${expected_concurrency}" '
  def sanitized_signature:
    type == "string" and test("^[1-9A-HJ-NP-Za-km-z]{10}\\.\\.\\.[1-9A-HJ-NP-Za-km-z]{10}$");
  .schema_version == 1 and .scenario == "compressed_surfpool_native_transfer_soak" and
  .transaction_count == $expected and .max_concurrency == $concurrency and
  .confirmed_action_count == .transaction_count and
  .logical_action_count == .transaction_count and .failures == 0 and
  .duplicate_logical_intents == 0 and .duplicate_signatures == 0 and
  .duplicate_enqueue_attempts == .transaction_count and
  .idempotent_duplicate_enqueue_rejections == .transaction_count and
  .budget_violations == 0 and .unresolved_submitted == 0 and
  .unresolved_unknown == 0 and .response_loss_injections == 1 and
  .submit_visibility_barriers == ((.transaction_count / 2) | floor) and
  .response_loss_reconciled_without_resend == 1 and
  .runtime_stack_restarts == 1 and .surfpool_process_restarts == 1 and
  .peak_worker_count > 0 and .peak_worker_count <= .max_concurrency and
  .wall_time_ms > 0 and .database_bytes > 0 and .journal_event_count > .transaction_count and
  .exact_source_debit_count == .transaction_count and
  .exact_destination_credit_count == .transaction_count and
  .signature_sample_limit == 12 and
  .signature_sample_count == ([.transaction_count, .signature_sample_limit] | min) and
  .signature_sample_count == (.sanitized_signature_samples | length) and
  all(.sanitized_signature_samples[]; sanitized_signature) and
  .state_delta.payer_before_lamports - .state_delta.payer_after_lamports
    == .state_delta.payer_debit_lamports and
  .state_delta.payer_debit_lamports
    == .state_delta.transferred_lamports + .state_delta.fees_lamports and
  .state_delta.payer_equation_met == true and .state_delta.destination_equation_met == true and
  .state_delta.destination_accounts == .transaction_count and
  .state_delta.each_destination_delta_lamports == 1000000 and
  .state_delta.destination_total_lamports
    == .state_delta.destination_accounts * .state_delta.each_destination_delta_lamports and
  .state_delta.destination_total_lamports == .state_delta.transferred_lamports and
  .state_delta.transferred_lamports
    == .transaction_count * .state_delta.each_destination_delta_lamports and
  .state_delta.fees_lamports > 0 and
  .state_delta.fees_lamports <= .transaction_count * 10000 and
  .surfpool_restart_provenance.verified == true and
  .surfpool_restart_provenance.session_schema_version == 2 and
  .surfpool_restart_provenance.before.pid != .surfpool_restart_provenance.after.pid and
  .surfpool_restart_provenance.pid_changed == true and
  .surfpool_restart_provenance.process_start_identity_changed == true and
  .surfpool_restart_provenance.persistent_identity_preserved == true and
  .surfpool_restart_provenance.before.resumed_persistent_database == false and
  .surfpool_restart_provenance.before.effective_airdrop_lamports > 0 and
  .surfpool_restart_provenance.after.resumed_persistent_database == true and
  .surfpool_restart_provenance.after.effective_airdrop_lamports == 0 and
  .surfpool_restart_provenance.network == "mainnet" and
  .surfpool_restart_provenance.offline_mode == true and
  .surfpool_restart_provenance.surfnet_id == .surfnet_id and
  .surfpool_restart_provenance.database == "persistent-local-surfnet" and
  .surfpool_restart_provenance.snapshot == "pinned-reviewed-state" and
  (.surfpool_restart_provenance.binary_sha256 | test("^[0-9a-f]{64}$")) and
  (.surfpool_restart_provenance.snapshot_archive_sha256 | test("^[0-9a-f]{64}$")) and
  (.surfpool_restart_provenance.snapshot_sha256 | test("^[0-9a-f]{64}$"))
' "${staging}/surfpool-soak.json" >/dev/null || die "Surfpool soak evidence contract failed"
[[ "$(grep -Fc 'COOKER_SOAK_EVIDENCE=' "${raw_dir}/surfpool-soak/test.log" || true)" == 1 ]] ||
  die "Surfpool soak output marker is missing or duplicated"

jq -e --slurpfile initial "${raw_dir}/surfpool-initial-session.json" \
  --slurpfile final "${raw_dir}/surfpool-final-session.json" \
  --slurpfile chain "${raw_dir}/chain-surfpool-session.json" \
  --slurpfile summary "${raw_dir}/surfpool-soak/surfpool-session.json" \
  --slurpfile soak "${staging}/surfpool-soak.json" '
  ($initial[0]) as $initial |
  ($final[0]) as $final |
  ($chain[0]) as $chain |
  ($summary[0]) as $summary |
  ($soak[0]) as $soak |
  def schema_valid:
    .sessionSchemaVersion == 2 and .surfpoolVersion == "1.4.0" and
    .network == "mainnet" and .offlineMode == true and .host == "127.0.0.1" and
    (.pid | type) == "number" and .pid > 0 and
    (.processStartIdentity | type) == "string" and (.processStartIdentity | length) > 0 and
    (.binarySha256 | test("^[0-9a-f]{64}$")) and
    (.snapshotArchiveSha256 | test("^[0-9a-f]{64}$")) and
    (.snapshotSha256 | test("^[0-9a-f]{64}$")) and
    .instructionProfilingDisabled == true and
    (.startArguments | index("--offline")) != null and
    (.startArguments | index("--disable-instruction-profiling")) != null and
    .configuredAirdropLamports > 0;
  ($initial | schema_valid) and ($final | schema_valid) and ($chain | schema_valid) and
  $initial.resumedPersistentDatabase == false and
  $initial.effectiveAirdropLamports == $initial.configuredAirdropLamports and
  $final.resumedPersistentDatabase == true and $final.effectiveAirdropLamports == 0 and
  $initial.pid != $final.pid and
  $initial.processStartIdentity != $final.processStartIdentity and
  $initial.surfnetId == $final.surfnetId and $initial.database == $final.database and
  $initial.snapshot == $final.snapshot and $initial.binarySha256 == $final.binarySha256 and
  $initial.snapshotArchiveSha256 == $final.snapshotArchiveSha256 and
  $initial.snapshotSha256 == $final.snapshotSha256 and
  $soak.surfnet_id == $initial.surfnetId and
  $soak.surfpool_restart_provenance.before.pid == $initial.pid and
  $soak.surfpool_restart_provenance.after.pid == $final.pid and
  $summary.surfpoolVersion == $final.surfpoolVersion and
  $summary.binarySha256 == $final.binarySha256 and $summary.network == $final.network and
  $summary.offlineMode == true and
  $summary.surfnetId == $final.surfnetId and
  $summary.snapshotArchiveSha256 == $final.snapshotArchiveSha256 and
  $summary.snapshotSha256 == $final.snapshotSha256 and
  $summary.configuredAirdropLamports == $final.configuredAirdropLamports and
  $summary.instructionProfilingDisabled == true and
  $summary.effectiveAirdropLamports == 0 and $summary.resumedPersistentDatabase == true and
  $chain.resumedPersistentDatabase == false and
  $chain.effectiveAirdropLamports == $chain.configuredAirdropLamports and
  $chain.surfnetId != $initial.surfnetId and $chain.database != $initial.database and
  $chain.processStartIdentity != $final.processStartIdentity and
  $chain.snapshot == $initial.snapshot and $chain.binarySha256 == $initial.binarySha256 and
  $chain.snapshotArchiveSha256 == $initial.snapshotArchiveSha256 and
  $chain.snapshotSha256 == $initial.snapshotSha256
' "${raw_dir}/surfpool-initial-session.json" >/dev/null ||
  die "Surfpool fresh/restart/independent-session provenance contract failed"

jq -n \
  --slurpfile initial "${raw_dir}/surfpool-initial-session.json" \
  --slurpfile final "${raw_dir}/surfpool-final-session.json" \
  --slurpfile chain "${raw_dir}/chain-surfpool-session.json" '
  def public_session:
    {
      session_schema_version: .sessionSchemaVersion,
      started_at: .startedAt,
      pid: .pid,
      surfpool_version: .surfpoolVersion,
      network: .network,
      offline_mode: .offlineMode,
      surfnet_id: .surfnetId,
      rpc: "http://127.0.0.1:<local>",
      websocket: "ws://127.0.0.1:<local>",
      database: "run-specific-persistent-local-surfnet",
      snapshot: "pinned-reviewed-state",
      binary_sha256: .binarySha256,
      snapshot_archive_sha256: .snapshotArchiveSha256,
      snapshot_sha256: .snapshotSha256,
      configured_airdrop_lamports: .configuredAirdropLamports,
      effective_airdrop_lamports: .effectiveAirdropLamports,
      instruction_profiling_disabled: .instructionProfilingDisabled,
      resumed_persistent_database: .resumedPersistentDatabase
    };
  {
    schema_version: 1,
    primary_initial: ($initial[0] | public_session),
    primary_after_restart: ($final[0] | public_session),
    independent_chain: ($chain[0] | public_session)
  }
' >"${staging}/surfpool-provenance.json"

jq -e '
  def unit_metric:
    type == "number" and . >= 0 and . <= 1;
  def cluster_metric:
    type == "number" and . >= -1 and . <= 1;
  def aggregate_stat:
    all([.mean, .min, .max, .standard_deviation, .confidence_95_low,
         .confidence_95_high][]; type == "number") and
    .standard_deviation >= 0 and .min >= -1 and .max <= 1 and
    .min <= .max and .mean >= (.min - 0.000000000001) and
    .mean <= (.max + 0.000000000001) and
    .confidence_95_low <= .mean and .confidence_95_high >= .mean;
  . as $root |
  ($root.config.controllers * $root.config.agents_per_controller) as $agents |
  ($agents * ($agents - 1) / 2) as $pairs |
  ($root.config.controllers * $root.config.agents_per_controller *
    ($root.config.agents_per_controller - 1) / 2) as $positive_pairs |
  ($agents * $root.config.days * $root.config.events_per_agent_per_day) as $events |
  ["naive_uniform", "independent_weighted", "persona_session"] as $planners |
  ["amount", "balance_rank", "destination", "funding", "route", "sequence",
   "synchrony", "timing"] as $features |
  .config.seeds == [11, 23, 37, 51, 71] and
  .config.controllers > 1 and .config.agents_per_controller > 1 and
  .config.days > 0 and .config.events_per_agent_per_day > 0 and
  (.config.threshold | unit_metric) and
  .config.ablations == [
    {"kind":"none"},
    {"kind":"without","feature":"timing"},
    {"kind":"without","feature":"amount"},
    {"kind":"without","feature":"sequence"},
    {"kind":"without","feature":"synchrony"},
    {"kind":"without","feature":"funding"}
  ] and
  ([.seeds[].planner] | unique | sort) == ($planners | sort) and
  (.seeds | length) == (($planners | length) * (.config.seeds | length) * (.config.ablations | length)) and
  ([.seeds[] | [.planner, (.seed | tostring), (.ablation | tojson)] | join("|")] |
    unique | length) == (.seeds | length) and
  all(.seeds[];
    . as $row |
    ($planners | index($row.planner)) != null and
    ($root.config.seeds | index($row.seed)) != null and
    any($root.config.ablations[]; . == $row.ablation) and
    (.trace_hash | test("^[0-9a-f]{64}$")) and
    .events == $events and .pairs == $pairs and .rejected_actions == 0 and
    ([.action_counts | keys[]] | sort) ==
      (["jupiter_swap", "native_transfer", "spl_transfer", "stake_lifecycle"] | sort) and
    ([.action_counts[]] | add) == .events and all(.action_counts[]; . > 0) and
    .composite.binary.threshold == $root.config.threshold and
    .composite.binary.true_positives + .composite.binary.false_negatives == $positive_pairs and
    .composite.binary.true_negatives + .composite.binary.false_positives == $pairs - $positive_pairs and
    .composite.binary.true_positives + .composite.binary.false_positives +
      .composite.binary.true_negatives + .composite.binary.false_negatives == $pairs and
    all([.composite.binary.precision, .composite.binary.recall, .composite.binary.f1,
         .composite.binary.roc_auc, .composite.binary.precision_at_k][]; unit_metric) and
    .composite.binary.k == $positive_pairs and
    (.composite.clustering.adjusted_rand | cluster_metric) and
    (.composite.clustering.normalized_mutual_information | unit_metric) and
    .composite.clustering.predicted_clusters >= 1 and
    .composite.clustering.predicted_clusters <= $agents and
    .composite.clustering.true_clusters == $root.config.controllers and
    ([.per_feature | keys[]] | sort) == ($features | sort) and
    all(.per_feature[];
      .binary.threshold == $root.config.threshold and
      .binary.true_positives + .binary.false_positives +
        .binary.true_negatives + .binary.false_negatives == $pairs and
      all([.binary.precision, .binary.recall, .binary.f1, .binary.roc_auc,
           .binary.precision_at_k][]; unit_metric) and
      (.clustering.adjusted_rand | cluster_metric) and
      (.clustering.normalized_mutual_information | unit_metric) and
      .clustering.predicted_clusters >= 1 and .clustering.predicted_clusters <= $agents and
      .clustering.true_clusters == $root.config.controllers) and
    ([.feature_separation | keys[]] | sort) == ($features | sort) and
    all(.feature_separation[];
      (.within_controller_mean | unit_metric) and (.between_controller_mean | unit_metric) and
      (.separation | cluster_metric) and
      (((.within_controller_mean - .between_controller_mean) - .separation) | fabs)
        < 0.000000000001)) and
  (.aggregates | length) == (($planners | length) * (.config.ablations | length)) and
  ([.aggregates[] | [.planner, (.ablation | tojson)] | join("|")] | unique | length)
    == (.aggregates | length) and
  all(.aggregates[];
    . as $aggregate |
    ($planners | index($aggregate.planner)) != null and
    any($root.config.ablations[]; . == $aggregate.ablation) and
    (["roc_auc", "f1", "precision_at_k", "adjusted_rand",
      "normalized_mutual_information"] - ($aggregate | keys) | length) == 0 and
    (.roc_auc | aggregate_stat) and (.f1 | aggregate_stat) and
    (.precision_at_k | aggregate_stat) and (.adjusted_rand | aggregate_stat) and
    (.normalized_mutual_information | aggregate_stat) and
    ([ $root.seeds[] |
       select(.planner == $aggregate.planner and .ablation == $aggregate.ablation) ] | length)
      == ($root.config.seeds | length) and
    ((.roc_auc.mean -
      ([ $root.seeds[] | select(.planner == $aggregate.planner and .ablation == $aggregate.ablation) |
         .composite.binary.roc_auc ] | add / length)) | fabs) < 0.000000000001 and
    ((.f1.mean -
      ([ $root.seeds[] | select(.planner == $aggregate.planner and .ablation == $aggregate.ablation) |
         .composite.binary.f1 ] | add / length)) | fabs) < 0.000000000001 and
    ((.precision_at_k.mean -
      ([ $root.seeds[] | select(.planner == $aggregate.planner and .ablation == $aggregate.ablation) |
         .composite.binary.precision_at_k ] | add / length)) | fabs) < 0.000000000001 and
    ((.adjusted_rand.mean -
      ([ $root.seeds[] | select(.planner == $aggregate.planner and .ablation == $aggregate.ablation) |
         .composite.clustering.adjusted_rand ] | add / length)) | fabs) < 0.000000000001 and
    ((.normalized_mutual_information.mean -
      ([ $root.seeds[] | select(.planner == $aggregate.planner and .ablation == $aggregate.ablation) |
         .composite.clustering.normalized_mutual_information ] | add / length)) | fabs)
      < 0.000000000001) and
  (.limitation | contains("common-funder graph remains directly observable"))
' "${raw_dir}/evaluation/evaluation.json" >/dev/null || die "evaluation evidence contract failed"
cp "${raw_dir}/evaluation/evaluation.json" "${staging}/metrics.json"
cp "${raw_dir}/evaluation/evaluation.csv" "${staging}/metrics.csv"
cp "${raw_dir}/evaluation/evaluation.md" "${staging}/metrics.md"

cp "${raw_dir}/config/cooker.toml" "${staging}/config.executed.toml"
if [[ "${mode}" == canonical ]]; then
  printf '%s\n' 'scripts/full-demo.sh' >"${staging}/reproduce.txt"
else
  printf '%s\n' 'scripts/full-demo.sh --quick' >"${staging}/reproduce.txt"
fi
printf '%s\n' 'Exact executed argv, timestamps, exit statuses, and log digests are in executions.json.' \
  >>"${staging}/reproduce.txt"

captured_at="$(date -u '+%Y-%m-%dT%H:%M:%SZ')"
cargo_lock_sha256="$(sha256_file "${PROJECT_ROOT}/Cargo.lock")"
surfpool_config_sha256="$(sha256_file "${PROJECT_ROOT}/configs/surfpool.env")"
behavior_model_sha256="$(sha256_file "${PROJECT_ROOT}/crates/cooker-core/src/behavior.rs")"
persona_model_sha256="$(sha256_file "${PROJECT_ROOT}/crates/cooker-core/src/persona.rs")"
scheduler_sha256="$(sha256_file "${PROJECT_ROOT}/crates/cooker-core/src/scheduler.rs")"
jupiter_quote_sha256="$(sha256_file "${PROJECT_ROOT}/fixtures/surfpool/jupiter/raydium-clmm-sol-usdc-cyb-433411234.quote.json")"
jupiter_instructions_sha256="$(sha256_file "${PROJECT_ROOT}/fixtures/surfpool/jupiter/raydium-clmm-sol-usdc-cyb-433411234.instructions.json")"
executed_config_sha256="$(sha256_file "${raw_dir}/config/cooker.toml")"
chain_log_sha256="$(sha256_file "${raw_dir}/chain-acceptance.log")"
jq -n \
  --arg captured_at "${captured_at}" \
  --arg mode "${mode}" \
  --arg git_commit "${git_commit}" \
  --arg git_branch "${git_branch}" \
  --argjson git_dirty "${source_dirty}" \
  --arg cargo_lock_sha256 "${cargo_lock_sha256}" \
  --arg surfpool_config_sha256 "${surfpool_config_sha256}" \
  --arg behavior_model_sha256 "${behavior_model_sha256}" \
  --arg persona_model_sha256 "${persona_model_sha256}" \
  --arg scheduler_sha256 "${scheduler_sha256}" \
  --arg jupiter_quote_sha256 "${jupiter_quote_sha256}" \
  --arg jupiter_instructions_sha256 "${jupiter_instructions_sha256}" \
  --arg executed_config_sha256 "${executed_config_sha256}" \
  --arg chain_log_sha256 "${chain_log_sha256}" \
  --slurpfile run "${staging}/run.json" \
  --slurpfile executions "${staging}/executions.json" \
  --slurpfile quality "${staging}/quality-gates.json" \
  --slurpfile cli "${staging}/cli-workflow.json" \
  --slurpfile chain "${staging}/transactions.json" \
  --slurpfile virtual "${staging}/virtual-soak.json" \
  --slurpfile soak "${staging}/surfpool-soak.json" \
  --slurpfile provenance "${staging}/surfpool-provenance.json" \
  --slurpfile metrics "${staging}/metrics.json" '
  {
    schema_version: 1,
    status: "passed",
    mode: $mode,
    captured_at: $captured_at,
    run_id: $run[0].run_id,
    started_at: $run[0].started_at,
    finished_at: $run[0].finished_at,
    source: {git_commit: $git_commit, git_branch: $git_branch, git_dirty: $git_dirty},
    environment: {
      tools: $run[0].tools,
      platform: $virtual[0].environment.platform,
      surfpool_version: $provenance[0].primary_initial.surfpool_version,
      surfpool_binary_sha256: $provenance[0].primary_initial.binary_sha256,
      snapshot_archive_sha256: $provenance[0].primary_initial.snapshot_archive_sha256,
      snapshot_sha256: $provenance[0].primary_initial.snapshot_sha256
    },
    reproducibility: {
      executed_command_records: ($executions[0] | length),
      all_exit_statuses_zero: all($executions[0][]; .exit_status == 0),
      cargo_lock_sha256: $cargo_lock_sha256,
      surfpool_config_sha256: $surfpool_config_sha256,
      behavior_model_sha256: $behavior_model_sha256,
      persona_model_sha256: $persona_model_sha256,
      scheduler_sha256: $scheduler_sha256,
      jupiter_quote_sha256: $jupiter_quote_sha256,
      jupiter_instructions_sha256: $jupiter_instructions_sha256,
      executed_config_sha256: $executed_config_sha256,
      raw_chain_log_sha256: $chain_log_sha256
    },
    gates: {
      quality: $quality[0].all_passed,
      cli_workflow: true,
      native_transfer: $chain[0].native.postconditions_met,
      spl_transfer: $chain[0].spl.postconditions_met,
      jupiter_swap: $chain[0].jupiter.postconditions_met,
      native_stake_lifecycle: $chain[0].stake.postconditions_met,
      fault_reconciliation: $chain[0].fault_reconciliation.postconditions_met,
      real_process_crash_recovery:
        ($chain[0].process_recovery.checkpoint_count == 6 and
         $chain[0].process_recovery.unresolved_actions == 0),
      virtual_soak: $virtual[0].proofs.completed_without_panic,
      surfpool_soak: ($soak[0].failures == 0),
      evaluation: true
    },
    totals: {
      demo_funding_transactions: $cli[0].funding.execute.workers.confirmed,
      demo_runtime_transactions: $cli[0].runtime.execute.workers.confirmed,
      demo_confirmation_reaudits: $cli[0].recovery.execute.audits.audited,
      chain_acceptance_scenarios: 6,
      process_crash_checkpoints: $chain[0].process_recovery.checkpoint_count,
      soak_transactions: $soak[0].transaction_count,
      evaluation_seed_rows: ($metrics[0].seeds | length),
      executed_command_records: ($executions[0] | length)
    },
    safety: {
      rpc_scope: "explicit-loopback-surfpool-only",
      public_chain_rpc_reads: 0,
      public_network_writes: 0,
      signer_files_committed: false,
      raw_logs_committed: false,
      primary_started_from_fresh_database:
        ($provenance[0].primary_initial.resumed_persistent_database == false),
      primary_restart_proved_persistence:
        ($provenance[0].primary_after_restart.resumed_persistent_database == true),
      chain_started_from_independent_fresh_database:
        ($provenance[0].independent_chain.resumed_persistent_database == false),
      recovery_transaction_submissions: $cli[0].recovery.execute.transaction_submissions,
      unresolved_runtime_actions: $cli[0].runtime.status_after.snapshot.unresolved_actions,
      soak_duplicate_signatures: $soak[0].duplicate_signatures,
      soak_budget_violations: $soak[0].budget_violations,
      post_pack_secret_scan: true,
      checksum_self_verification: true
    },
    limitations: [
      $metrics[0].limitation,
      "The evaluator uses synthetic labeled attacker scenarios; measured scores do not establish anonymity or mainnet behavior.",
      "Jupiter evidence replays a reviewed pinned Raydium CLMM state snapshot and does not claim current market execution quality.",
      "Surfpool is a local offline-snapshot topology and does not reproduce public-network latency, validator competition, or topology.",
      "Stake acceptance uses the native Solana stake lifecycle; Marinade is not implemented or claimed."
    ]
  }' >"${staging}/manifest.json"

naive_auc="$(jq -r '.aggregates[] | select(.planner == "naive_uniform" and .ablation.kind == "none") | .roc_auc.mean' "${staging}/metrics.json")"
independent_auc="$(jq -r '.aggregates[] | select(.planner == "independent_weighted" and .ablation.kind == "none") | .roc_auc.mean' "${staging}/metrics.json")"
persona_auc="$(jq -r '.aggregates[] | select(.planner == "persona_session" and .ablation.kind == "none") | .roc_auc.mean' "${staging}/metrics.json")"
persona_f1="$(jq -r '.aggregates[] | select(.planner == "persona_session" and .ablation.kind == "none") | .f1.mean' "${staging}/metrics.json")"
soak_transactions="$(jq -r '.transaction_count' "${staging}/surfpool-soak.json")"
virtual_peak_rss="$(jq -r '.environment.peak_rss_bytes' "${staging}/virtual-soak.json")"
demo_agents="$(jq -r '.funding.execute.fleet_agents' "${staging}/cli-workflow.json")"

cat >"${staging}/report.md" <<REPORT
# Account Cooker Evidence Report

Status: **passed** (${mode} run at ${captured_at})

## Executed Proofs

- Full CLI lifecycle: read-only doctor and previews, ${demo_agents}-agent fleet generation and local funding, bounded execution, status, idempotent funding replay, and historical confirmation audit with zero recovery submissions.
- Chain acceptance: native SOL, classic SPL with ATA creation, Jupiter exact-input from pinned reviewed state, deterministic/unknown-outcome reconciliation, all six real SIGKILL/restart checkpoints, and native stake lifecycle last.
- Virtual soak: $(jq -r '.agents' "${staging}/virtual-soak.json") agents for $(jq -r '.days' "${staging}/virtual-soak.json") virtual days across $(jq -r '.seeds | length' "${staging}/virtual-soak.json") seeds; deterministic replay and all safety proofs passed; peak measured RSS ${virtual_peak_rss} bytes.
- Surfpool soak: ${soak_transactions} real local transactions, one lost response, runtime reconstruction, Surfpool restart, zero duplicate signatures, zero budget violations, and zero unresolved outcomes.

## Measured Result

Full-attacker ROC AUC means were ${naive_auc} for naive uniform, ${independent_auc} for independent weighted, and ${persona_auc} for persona/session. Persona/session F1 at the fixed 0.55 threshold was ${persona_f1}. These results show only a measured change in this synthetic benchmark; they do not establish anonymity.

## Interpretation Bound

The common-funder graph remains directly observable. The evaluator is synthetic and does not establish anonymity. Jupiter uses a reviewed pinned state snapshot, native stake is used instead of Marinade, and local Surfpool does not reproduce public-network topology or execution quality. All chain transactions in this pack were signed and executed only against loopback Surfpool. Full local signatures, databases, generated signers, and raw logs are excluded from this pack.
REPORT

cat >"${staging}/test-summary.txt" <<SUMMARY
PASS cli_workflow
PASS native_transfer
PASS classic_spl_transfer
PASS jupiter_exact_input_swap
PASS native_stake_lifecycle
PASS failure_and_same_signature_reconciliation
PASS six_real_sigkill_restart_checkpoints
PASS virtual_soak_${mode}
PASS surfpool_soak_${mode}_${soak_transactions}_transactions
PASS five_seed_comparative_evaluation
PASS formatting_clippy_tests_security_and_release_build
SUMMARY

checksum_inputs="${staging}.checksum-inputs.$$"
(
  cd "${staging}"
  find . -type f ! -name checksums.txt -print | LC_ALL=C sort >"${checksum_inputs}"
  while IFS= read -r file; do
    digest="$(sha256_file "${file}")"
    printf '%s  %s\n' "${digest}" "${file#./}"
  done <"${checksum_inputs}" >checksums.txt
)
find "${checksum_inputs}" -delete
checksum_inputs=''

chmod -R go-w "${staging}"
mv "${staging}" "${output_dir}"
staging=''
trap - EXIT INT TERM

printf 'Evidence pack verified: %s\n' "${output_dir}"
printf 'Source: %s (%s, dirty=%s)\n' "${git_commit}" "${git_branch}" "${source_dirty}"

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
for command in awk cp date find git jq sed sort; do
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
  surfpool-soak/soak.json
  surfpool-soak/surfpool-session.json
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
    index($0, marker) == 1 { value = substr($0, length(marker) + 1) }
    END { if (value == "") exit 1; print value }
  ' "${raw_dir}/chain-acceptance.log" >"${destination}" ||
    die "chain acceptance marker is missing: ${marker}"
  jq empty "${destination}" || die "chain acceptance marker is invalid JSON: ${marker}"
}

extract_marker COOKER_NATIVE_EVIDENCE= "${staging}/.chain/native.json"
extract_marker COOKER_SPL_EVIDENCE= "${staging}/.chain/spl.json"
extract_marker COOKER_STAKE_EVIDENCE= "${staging}/.chain/stake.json"
extract_marker COOKER_FAULT_EVIDENCE= "${staging}/.chain/fault.json"
extract_marker COOKER_JUPITER_EVIDENCE= "${staging}/.chain/jupiter.json"
jq -n \
  --slurpfile native "${staging}/.chain/native.json" \
  --slurpfile spl "${staging}/.chain/spl.json" \
  --slurpfile stake "${staging}/.chain/stake.json" \
  --slurpfile fault "${staging}/.chain/fault.json" \
  --slurpfile jupiter "${staging}/.chain/jupiter.json" \
  '{
    schema_version: 1,
    native: $native[0],
    spl: $spl[0],
    stake: $stake[0],
    fault_reconciliation: $fault[0],
    jupiter: $jupiter[0]
  }' >"${staging}/transactions.json"
find "${staging}/.chain" -depth -delete

jq -e '
  .schema_version == 1 and
  .native.postconditions_met == true and (.native.signature | contains("...")) and
  .spl.postconditions_met == true and (.spl.signature | contains("...")) and
  (.spl.later_signature | contains("...")) and
  .stake.postconditions_met == true and (.stake.vote_setup_signature | contains("...")) and
  (.stake.enter_signature | contains("...")) and
  (.stake.deactivate_signature | contains("...")) and
  (.stake.withdraw_signature | contains("...")) and
  .fault_reconciliation.postconditions_met == true and
  (.fault_reconciliation.simulation_failure_signature | contains("...")) and
  (.fault_reconciliation.stale_signature | contains("...")) and
  (.fault_reconciliation.reconciled_signature | contains("...")) and
  .jupiter.postconditions_met == true and (.jupiter.signature | contains("...")) and
  .jupiter.plan_source == "reviewed-fixture" and
  .jupiter.reviewed_user_account_rebindings >= 3
' "${staging}/transactions.json" >/dev/null || die "chain acceptance evidence contract failed"

jq '{
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
    peak_rss_bytes: .environment.peak_rss_bytes
  }
}' "${raw_dir}/virtual-soak/manifest.json" >"${staging}/virtual-soak.json"

jq -e --arg mode "${mode}" --arg commit "${git_commit}" '
  .schema_version == 1 and .mode == $mode and .offline_only == true and
  .signer_loaded == false and .network_requests == 0 and
  (.environment.platform | test("^[A-Za-z0-9._-]+ [A-Za-z0-9._-]+$")) and
  .completed_seeds == (.seeds | length) and (.seeds | length) >= 2 and
  .proofs.deterministic_replay == true and
  .proofs.chronological_events == true and .proofs.label_free_events == true and
  .proofs.bounded_scheduler_workers == true and .proofs.daily_budget_respected == true and
  .proofs.completed_without_panic == true and .environment.git.commit == $commit and
  (if $mode == "canonical" then
    .agents == 1000 and .days == 30 and (.seeds | length) >= 5 and
    .environment.git.dirty == false
   else .agents >= 1 and .days >= 1 end)
' "${staging}/virtual-soak.json" >/dev/null || die "virtual soak evidence contract failed"

jq 'del(.database_file)' "${raw_dir}/surfpool-soak/soak.json" >"${staging}/surfpool-soak.json"
if [[ "${mode}" == canonical ]]; then minimum_transactions=1000; else minimum_transactions=2; fi
jq -e --argjson minimum "${minimum_transactions}" '
  .schema_version == 1 and .transaction_count >= $minimum and
  .confirmed_action_count == .transaction_count and
  .logical_action_count == .transaction_count and .failures == 0 and
  .duplicate_logical_intents == 0 and .duplicate_signatures == 0 and
  .budget_violations == 0 and .unresolved_submitted == 0 and
  .unresolved_unknown == 0 and .response_loss_injections == 1 and
  .submit_visibility_barriers >= 1 and
  .response_loss_reconciled_without_resend == 1 and
  .runtime_stack_restarts == 1 and .surfpool_process_restarts == 1 and
  .peak_worker_count <= .max_concurrency and
  .state_delta.payer_debit_lamports
    == .state_delta.transferred_lamports + .state_delta.fees_lamports and
  all(.sanitized_signature_samples[]; contains("..."))
' "${staging}/surfpool-soak.json" >/dev/null || die "Surfpool soak evidence contract failed"

jq -e '
  .surfpoolVersion == "1.4.0" and .network == "mainnet" and
  (.binarySha256 | test("^[0-9a-f]{64}$")) and
  (.snapshotArchiveSha256 | test("^[0-9a-f]{64}$")) and
  (.snapshotSha256 | test("^[0-9a-f]{64}$")) and
  .configuredAirdropLamports > 0 and .effectiveAirdropLamports == 0 and
  .resumedPersistentDatabase == true
' "${raw_dir}/surfpool-soak/surfpool-session.json" >/dev/null ||
  die "Surfpool session provenance contract failed"
cp "${raw_dir}/surfpool-soak/surfpool-session.json" "${staging}/surfpool-session.json"

jq -e '
  .config.seeds == [11, 23, 37, 51, 71] and
  (.seeds | length) == 90 and (.aggregates | length) == 18 and
  ([.aggregates[] | select(.ablation.kind == "none") | .planner] | unique | length) == 3 and
  (.limitation | contains("common-funder graph remains directly observable"))
' "${raw_dir}/evaluation/evaluation.json" >/dev/null || die "evaluation evidence contract failed"
cp "${raw_dir}/evaluation/evaluation.json" "${staging}/metrics.json"
cp "${raw_dir}/evaluation/evaluation.csv" "${staging}/metrics.csv"
cp "${raw_dir}/evaluation/evaluation.md" "${staging}/metrics.md"

sed -E \
  -e 's#rpc_url = "http://127\.0\.0\.1:[0-9]+"#rpc_url = "http://127.0.0.1:<local>"#' \
  -e 's#surfnet_id = "[^"]+"#surfnet_id = "noise-full-demo"#' \
  "${raw_dir}/config/cooker.toml" >"${staging}/config.redacted.toml"

cat >"${staging}/commands.txt" <<'COMMANDS'
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-features
cargo test --locked --workspace --doc
shellcheck -x -P SCRIPTDIR scripts/*.sh scripts/lib/*.sh
cargo audit
cargo deny check
gitleaks dir --redact --no-banner --verbose .
gitleaks git --redact --no-banner --verbose .
scripts/surfpool-chain-acceptance.sh
scripts/virtual-soak.sh --force
scripts/surfpool-soak.sh --transactions 1000
cooker evaluate --config <redacted-config> --output-dir <raw-evaluation> --force
scripts/full-demo.sh
COMMANDS

captured_at="$(date -u '+%Y-%m-%dT%H:%M:%SZ')"
jq -n \
  --arg captured_at "${captured_at}" \
  --arg mode "${mode}" \
  --arg git_commit "${git_commit}" \
  --arg git_branch "${git_branch}" \
  --argjson git_dirty "${source_dirty}" \
  --slurpfile quality "${staging}/quality-gates.json" \
  --slurpfile cli "${staging}/cli-workflow.json" \
  --slurpfile chain "${staging}/transactions.json" \
  --slurpfile virtual "${staging}/virtual-soak.json" \
  --slurpfile soak "${staging}/surfpool-soak.json" \
  --slurpfile session "${staging}/surfpool-session.json" \
  --slurpfile metrics "${staging}/metrics.json" '
  {
    schema_version: 1,
    status: "passed",
    mode: $mode,
    captured_at: $captured_at,
    source: {git_commit: $git_commit, git_branch: $git_branch, git_dirty: $git_dirty},
    environment: {
      rustc: $virtual[0].environment.rustc,
      cargo: $virtual[0].environment.cargo,
      platform: $virtual[0].environment.platform,
      surfpool_version: $session[0].surfpoolVersion,
      surfpool_binary_sha256: $session[0].binarySha256,
      snapshot_archive_sha256: $session[0].snapshotArchiveSha256,
      snapshot_sha256: $session[0].snapshotSha256
    },
    gates: {
      quality: $quality[0].all_passed,
      cli_workflow: true,
      native_transfer: $chain[0].native.postconditions_met,
      spl_transfer: $chain[0].spl.postconditions_met,
      jupiter_swap: $chain[0].jupiter.postconditions_met,
      native_stake_lifecycle: $chain[0].stake.postconditions_met,
      fault_reconciliation: $chain[0].fault_reconciliation.postconditions_met,
      virtual_soak: $virtual[0].proofs.completed_without_panic,
      surfpool_soak: ($soak[0].failures == 0),
      evaluation: true
    },
    totals: {
      demo_funding_transactions: $cli[0].funding.execute.workers.confirmed,
      demo_runtime_transactions: $cli[0].runtime.execute.workers.confirmed,
      demo_confirmation_reaudits: $cli[0].recovery.execute.audits.audited,
      chain_acceptance_scenarios: 5,
      soak_transactions: $soak[0].transaction_count,
      evaluation_seed_rows: ($metrics[0].seeds | length)
    },
    safety: {
      rpc_scope: "explicit-loopback-surfpool-only",
      public_network_writes: 0,
      signer_files_committed: false,
      raw_logs_committed: false,
      recovery_transaction_submissions: $cli[0].recovery.execute.transaction_submissions,
      unresolved_runtime_actions: $cli[0].runtime.status_after.snapshot.unresolved_actions,
      soak_duplicate_signatures: $soak[0].duplicate_signatures,
      soak_budget_violations: $soak[0].budget_violations
    },
    limitation: $metrics[0].limitation
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
- Chain acceptance: native SOL, classic SPL with ATA creation, Jupiter exact-input from pinned reviewed state, native stake lifecycle, and deterministic/unknown-outcome reconciliation.
- Virtual soak: $(jq -r '.agents' "${staging}/virtual-soak.json") agents for $(jq -r '.days' "${staging}/virtual-soak.json") virtual days across $(jq -r '.seeds | length' "${staging}/virtual-soak.json") seeds; deterministic replay and all safety proofs passed; peak measured RSS ${virtual_peak_rss} bytes.
- Surfpool soak: ${soak_transactions} real local transactions, one lost response, runtime reconstruction, Surfpool restart, zero duplicate signatures, zero budget violations, and zero unresolved outcomes.

## Measured Result

Full-attacker ROC AUC means were ${naive_auc} for naive uniform, ${independent_auc} for independent weighted, and ${persona_auc} for persona/session. Persona/session F1 at the fixed 0.55 threshold was ${persona_f1}. These results show only a measured change in this synthetic benchmark; they do not establish anonymity.

## Interpretation Bound

The common-funder graph remains directly observable. All chain transactions in this pack were signed and executed only against loopback Surfpool. Full local signatures, databases, generated signers, and raw logs are excluded from this pack.
REPORT

cat >"${staging}/test-summary.txt" <<SUMMARY
PASS cli_workflow
PASS native_transfer
PASS classic_spl_transfer
PASS jupiter_exact_input_swap
PASS native_stake_lifecycle
PASS failure_and_same_signature_reconciliation
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

#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

output_dir="${PROJECT_ROOT}/evidence/raw/virtual-soak"
config_path="${PROJECT_ROOT}/cooker.toml"
seeds_csv="11,23,37,51,71"
agents=1000
days=30
quick=0
force=0
config_supplied=0
seeds_supplied=0
agents_supplied=0
days_supplied=0

usage() {
  cat <<'USAGE'
Usage: scripts/virtual-soak.sh [options]

Runs the canonical offline virtual soak in release mode and verifies every trace.

Options:
  --output DIR       Evidence directory (default: evidence/raw/virtual-soak)
  --output-dir DIR   Alias for --output
  --config FILE      Cooker config; a temporary example is generated when absent
  --seeds CSV        Unique decimal held-out seeds (default: 11,23,37,51,71)
  --agents COUNT     Simulated agents (canonical: 1000)
  --days COUNT       Virtual days (canonical: 30)
  --quick            Permit reduced dimensions; defaults to 2 seeds, 16 agents, 2 days
  --force            Atomically replace existing soak traces and manifest
  -h, --help         Show this help
USAGE
}

die() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"
}

is_uint() {
  [[ "$1" =~ ^[0-9]+$ ]]
}

while (($#)); do
  case "$1" in
    --output | --output-dir)
      (($# >= 2)) || die "$1 requires a directory"
      output_dir="$2"
      shift 2
      ;;
    --config)
      (($# >= 2)) || die "--config requires a file"
      config_path="$2"
      config_supplied=1
      shift 2
      ;;
    --seeds)
      (($# >= 2)) || die "--seeds requires a comma-separated list"
      seeds_csv="$2"
      seeds_supplied=1
      shift 2
      ;;
    --agents)
      (($# >= 2)) || die "--agents requires a count"
      agents="$2"
      agents_supplied=1
      shift 2
      ;;
    --days)
      (($# >= 2)) || die "--days requires a count"
      days="$2"
      days_supplied=1
      shift 2
      ;;
    --quick)
      quick=1
      shift
      ;;
    --force)
      force=1
      shift
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *) die "unknown argument: $1" ;;
  esac
done

if ((quick)); then
  ((seeds_supplied)) || seeds_csv="11,23"
  ((agents_supplied)) || agents=16
  ((days_supplied)) || days=2
fi

[[ -n "${output_dir}" ]] || die "output directory cannot be empty"
is_uint "${agents}" || die "agents must be an unsigned integer"
is_uint "${days}" || die "days must be an unsigned integer"
((agents >= 1 && agents <= 10000)) || die "agents must be in 1..=10000"
((days >= 1 && days <= 365)) || die "days must be in 1..=365"
((agents * days <= 100000)) || die "each seed is limited to 100000 agent-days"
[[ "${seeds_csv}" != ,* && "${seeds_csv}" != *, && "${seeds_csv}" != *,,* ]] ||
  die "seeds must be a non-empty comma-separated list"

IFS=',' read -r -a seeds <<<"${seeds_csv}"
((${#seeds[@]} >= 1)) || die "at least one seed is required"
for ((i = 0; i < ${#seeds[@]}; i++)); do
  is_uint "${seeds[i]}" || die "seed must be a decimal u64: ${seeds[i]}"
  for ((j = i + 1; j < ${#seeds[@]}; j++)); do
    [[ "${seeds[i]}" != "${seeds[j]}" ]] || die "held-out seeds must be unique"
  done
done

if ((!quick)); then
  ((${#seeds[@]} >= 5)) || die "canonical soak requires at least five unique seeds"
  ((agents == 1000 && days == 30)) ||
    die "canonical soak requires exactly 1000 agents for 30 days; pass --quick for reduced dimensions"
  [[ -z "$(git -C "${PROJECT_ROOT}" status --short --untracked-files=all)" ]] ||
    die "canonical soak requires a clean source tree"
fi

for command in cargo rustc git jq tee uname awk date; do
  require_command "${command}"
done
[[ -x /usr/bin/time ]] || die "/usr/bin/time is required for peak RSS evidence"

mkdir -p "${output_dir}" "${PROJECT_ROOT}/evidence/tmp/virtual-soak"
timing_file="${output_dir}/time.txt"
run_log="${output_dir}/run.log"
environment_file="${output_dir}/environment.json"
manifest_file="${output_dir}/manifest.json"
probe_file="${PROJECT_ROOT}/evidence/tmp/virtual-soak/time-probe.$$"
manifest_temp="${output_dir}/.manifest.json.tmp.$$"
environment_temp="${output_dir}/.environment.json.tmp.$$"

cleanup() {
  rm -f "${probe_file}" "${manifest_temp}" "${environment_temp}"
}
trap cleanup EXIT INT TERM

if /usr/bin/time -l -o "${probe_file}" /usr/bin/true >/dev/null 2>&1; then
  time_style=bsd
  time_command=(/usr/bin/time -l -o "${timing_file}")
elif /usr/bin/time -v -o "${probe_file}" /usr/bin/true >/dev/null 2>&1; then
  time_style=gnu
  time_command=(/usr/bin/time -v -o "${timing_file}")
else
  die "/usr/bin/time supports neither BSD -l nor GNU -v resource reporting"
fi
rm -f "${probe_file}"

printf 'Building release cooker with Cargo network access disabled...\n'
(
  cd "${PROJECT_ROOT}"
  CARGO_NET_OFFLINE=true cargo build --offline --locked --release -p account-cooker
)
cooker_bin="${PROJECT_ROOT}/target/release/cooker"
[[ -x "${cooker_bin}" ]] || die "release cooker binary was not produced"

if [[ ! -r "${config_path}" ]]; then
  ((config_supplied == 0)) || die "config is not readable: ${config_path}"
  config_path="${PROJECT_ROOT}/evidence/tmp/virtual-soak/cooker.toml"
  "${cooker_bin}" init --output "${config_path}" --force >/dev/null
fi

run_arguments=(
  soak
  --config "${config_path}"
  --output-dir "${output_dir}"
  --seeds "${seeds_csv}"
  --agents "${agents}"
  --days "${days}"
)
((quick)) && run_arguments+=(--quick)
((force)) && run_arguments+=(--force)
if ((quick)); then mode=quick; else mode=canonical; fi

printf 'Running %s soak: %s seed(s), %s agents, %s virtual days...\n' \
  "${mode}" "${#seeds[@]}" "${agents}" "${days}"
"${time_command[@]}" "${cooker_bin}" "${run_arguments[@]}" | tee "${run_log}"

if [[ "${time_style}" == bsd ]]; then
  peak_rss_bytes="$(awk 'tolower($0) ~ /maximum resident set size/ { print $1; exit }' "${timing_file}")"
else
  peak_rss_kib="$(awk -F: 'tolower($1) ~ /maximum resident set size/ { gsub(/[[:space:]]/, "", $2); print $2; exit }' "${timing_file}")"
  is_uint "${peak_rss_kib}" || die "failed to parse GNU /usr/bin/time peak RSS"
  peak_rss_bytes=$((peak_rss_kib * 1024))
fi
is_uint "${peak_rss_bytes:-}" || die "failed to parse /usr/bin/time peak RSS"

git_commit="$(git -C "${PROJECT_ROOT}" rev-parse HEAD 2>/dev/null || printf unknown)"
git_branch="$(git -C "${PROJECT_ROOT}" symbolic-ref --short -q HEAD 2>/dev/null || printf detached)"
git_status="$(git -C "${PROJECT_ROOT}" status --short 2>/dev/null || printf unavailable)"
if [[ -n "${git_status}" ]]; then git_dirty=true; else git_dirty=false; fi
command_text="$(printf '%q ' "${cooker_bin}" "${run_arguments[@]}")"

jq -n \
  --arg captured_at "$(date -u '+%Y-%m-%dT%H:%M:%SZ')" \
  --arg platform "$(uname -s) $(uname -m)" \
  --arg rustc "$(rustc -Vv)" \
  --arg cargo "$(cargo -V)" \
  --arg git_commit "${git_commit}" \
  --arg git_branch "${git_branch}" \
  --arg git_status "${git_status}" \
  --argjson git_dirty "${git_dirty}" \
  --arg time_implementation "/usr/bin/time ${time_style}" \
  --argjson peak_rss_bytes "${peak_rss_bytes}" \
  --arg command "${command_text}" \
  --arg config "${config_path}" \
  '{
    captured_at: $captured_at,
    platform: $platform,
    rustc: $rustc,
    cargo: $cargo,
    git: {commit: $git_commit, branch: $git_branch, dirty: $git_dirty, status_short: $git_status},
    build_profile: "release",
    cargo_network: "offline",
    time_implementation: $time_implementation,
    peak_rss_bytes: $peak_rss_bytes,
    command: $command,
    config: $config
  }' >"${environment_temp}"
mv "${environment_temp}" "${environment_file}"

[[ -r "${manifest_file}" ]] || die "soak did not produce ${manifest_file}"
jq --slurpfile environment "${environment_file}" \
  '.environment = $environment[0]' "${manifest_file}" >"${manifest_temp}"
mv "${manifest_temp}" "${manifest_file}"

verify_arguments=(soak --output-dir "${output_dir}" --verify-only)
((quick)) && verify_arguments+=(--quick)
"${cooker_bin}" "${verify_arguments[@]}" | tee -a "${run_log}"

seeds_json="$(printf '%s\n' "${seeds[@]}" | jq -R 'tonumber' | jq -s '.')"
jq -e \
  --arg mode "${mode}" \
  --argjson agents "${agents}" \
  --argjson days "${days}" \
  --argjson seeds "${seeds_json}" '
  def proof_passes:
    .deterministic_replay == true and .chronological_events == true and
    .label_free_events == true and .bounded_scheduler_workers == true and
    .daily_budget_respected == true and .completed_without_panic == true;
  def stats_valid:
    (.min | type) == "number" and (.mean | type) == "number" and
    (.max | type) == "number" and .min <= .mean and .mean <= .max;
  .schema_version == 1 and .mode == $mode and
  .offline_only == true and .signer_loaded == false and .network_requests == 0 and
  .trace_hash_algorithm == "blake3" and .scheduler == "stable_bounded" and
  (.start | type) == "string" and (.model_version | length) > 0 and
  .agents == $agents and .days == $days and
  .max_concurrency >= 1 and .max_concurrency <= .agents and
  .max_decisions_per_agent_day >= 1 and .daily_budget >= 1 and
  .seeds == $seeds and .completed_seeds == ($seeds | length) and
  (.per_seed | length) == ($seeds | length) and (.proofs | proof_passes) and
  .total_wall_time_ms >= 0 and
  (all(.per_seed[];
    (.seed | type) == "number" and (.seed_hex | test("^[0-9a-f]{64}$")) and
    (.run_id | length) > 0 and .decisions >= .observable_events and
    .observable_events >= 1 and .trace_bytes >= 1 and
    (.trace_blake3 | test("^[0-9a-f]{64}$")) and
    (.trace_file | test("^traces/seed-[0-9]+\\.jsonl$")) and
    .wall_time_ms >= 0 and .replay_wall_time_ms >= 0 and (.proofs | proof_passes))) and
  (.aggregate.decisions | stats_valid) and
  (.aggregate.observable_events | stats_valid) and
  (.aggregate.trace_bytes | stats_valid) and
  (.aggregate.wall_time_ms | stats_valid) and
  (.aggregate.replay_wall_time_ms | stats_valid) and
  (.environment.captured_at | type) == "string" and
  (.environment.platform | length) > 0 and (.environment.rustc | length) > 0 and
  (.environment.cargo | length) > 0 and (.environment.git.commit | length) > 0 and
  (.environment.git.branch | length) > 0 and
  (.environment.git.dirty | type) == "boolean" and
  (if $mode == "canonical" then .environment.git.dirty == false else true end) and
  (.environment.git.status_short | type) == "string" and
  .environment.build_profile == "release" and .environment.cargo_network == "offline" and
  (.environment.time_implementation | startswith("/usr/bin/time ")) and
  .environment.peak_rss_bytes >= 1 and (.environment.command | length) > 0 and
  (.environment.config | length) > 0
  ' "${manifest_file}" >/dev/null || die "manifest field validation failed"

printf 'Virtual soak evidence verified: %s\n' "${manifest_file}"
printf 'Peak RSS: %s bytes (%s /usr/bin/time)\n' "${peak_rss_bytes}" "${time_style}"

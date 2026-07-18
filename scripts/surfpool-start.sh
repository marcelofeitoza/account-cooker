#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib/surfpool-common.sh
source "${SCRIPT_DIR}/lib/surfpool-common.sh"

surfpool_require_harness_commands
surfpool_require_command cargo
surfpool_require_command env
surfpool_require_command nohup
surfpool_validate_settings
surfpool_verify_install

mkdir -p "${SURFPOOL_KEYS_DIR}" "${SURFPOOL_STATE_DIR}" "${SURFPOOL_LOG_DIR}"
chmod 700 "${SURFPOOL_HOME}" "${SURFPOOL_KEYS_DIR}" "${SURFPOOL_STATE_DIR}" "${SURFPOOL_LOG_DIR}"
surfpool_prepare_snapshot

existing_pid="$(surfpool_read_pid || true)"
if [[ -n "${existing_pid}" ]]; then
  if surfpool_pid_matches "${existing_pid}"; then
    surfpool_verify_rpc
    surfpool_verify_runtime_env
    surfpool_note "Surfpool is already running (pid ${existing_pid}) at ${SURFPOOL_RPC_URL}"
    exit 0
  fi
  if surfpool_pid_is_live "${existing_pid}"; then
    surfpool_die "PID file points to a process this harness did not start: ${existing_pid}"
  fi
  surfpool_warn "removing stale PID file"
  rm -f "${SURFPOOL_PID_FILE}"
fi

surfpool_check_ports_free

if [[ -e "${SURFPOOL_DB}" && ! -e "${SURFPOOL_KEYPAIR}" ]]; then
  surfpool_die "persistent Surfpool database exists but its local funder key is missing"
fi

effective_airdrop_lamports="${SURFPOOL_AIRDROP_LAMPORTS}"
resumed_persistent_database=false
if [[ -e "${SURFPOOL_DB}" ]]; then
  [[ -r "${SURFPOOL_SESSION_FILE}" ]] ||
    surfpool_die "persistent Surfpool database exists without its session provenance"
  jq -e \
    --arg database "${SURFPOOL_DB}" \
    --arg network "${SURFPOOL_NETWORK}" \
    --arg surfnet_id "${SURFPOOL_ID}" \
    --arg snapshot_archive "${SURFPOOL_SNAPSHOT_ARCHIVE_SHA256}" \
    --arg snapshot "${SURFPOOL_SNAPSHOT_SHA256}" '
      .database == $database and .network == $network and .surfnetId == $surfnet_id and
      .snapshotArchiveSha256 == $snapshot_archive and .snapshotSha256 == $snapshot
    ' "${SURFPOOL_SESSION_FILE}" >/dev/null ||
    surfpool_die "persistent Surfpool session provenance does not match this harness"
  effective_airdrop_lamports=0
  resumed_persistent_database=true
elif [[ -e "${SURFPOOL_SESSION_FILE}" ]]; then
  surfpool_die "Surfpool session provenance exists without its persistent database"
fi

cargo run \
  --quiet \
  --locked \
  --manifest-path "${PROJECT_ROOT}/Cargo.toml" \
  -p account-cooker \
  -- keygen \
  --project-root "${PROJECT_ROOT}" \
  --output "${SURFPOOL_KEYPAIR}" \
  >/dev/null || surfpool_die "failed to load or generate local funder keypair"
surfpool_write_runtime_env

surfpool_build_start_command "${effective_airdrop_lamports}" ||
  surfpool_die "invalid effective Surfpool airdrop amount"
command=("${SURFPOOL_START_COMMAND[@]}")

{
  printf '\n[%s] launching:' "$(date -u '+%Y-%m-%dT%H:%M:%SZ')"
  printf ' %q' "${command[@]}"
  printf '\n'
} >>"${SURFPOOL_LAUNCHER_LOG}"

ready=0
pid=''
launched_process_start_identity=''
cleanup_startup() {
  local current_pid current_start_identity status=$?
  trap - EXIT INT TERM
  if ((ready == 0)) && [[ -n "${pid}" ]]; then
    if surfpool_pid_is_live "${pid}"; then
      current_start_identity="$(surfpool_process_start_identity "${pid}" 2>/dev/null || true)"
      if [[ -n "${launched_process_start_identity}" && \
        "${current_start_identity}" == "${launched_process_start_identity}" ]] &&
        surfpool_process_matches_start_command "${pid}" "${effective_airdrop_lamports}"; then
        kill -TERM "${pid}" 2>/dev/null || true
      else
        surfpool_warn "launched PID changed identity; refusing to signal ${pid} during cleanup"
      fi
    fi
    current_pid="$(surfpool_read_pid || true)"
    if [[ "${current_pid}" == "${pid}" ]]; then
      rm -f "${SURFPOOL_PID_FILE}"
    fi
    surfpool_warn "Surfpool did not become ready; see ${SURFPOOL_LAUNCHER_LOG}"
  fi
  exit "${status}"
}
trap cleanup_startup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

nohup env -u SURFPOOL_DATASOURCE_RPC_URL "${command[@]}" \
  </dev/null >>"${SURFPOOL_LAUNCHER_LOG}" 2>&1 &
pid=$!
launched_process_start_identity="$(surfpool_process_start_identity "${pid}")" ||
  surfpool_die "failed to identify the launched Surfpool process"

pid_temp="${SURFPOOL_PID_FILE}.tmp.$$"
printf '%s\n' "${pid}" >"${pid_temp}"
chmod 600 "${pid_temp}"
mv "${pid_temp}" "${SURFPOOL_PID_FILE}"

if ! surfpool_wait_for_rpc "${pid}"; then
  if [[ -r "${SURFPOOL_LAUNCHER_LOG}" ]]; then
    tail -n 40 "${SURFPOOL_LAUNCHER_LOG}" >&2 || true
  fi
  surfpool_die "Surfpool failed to become ready within ${SURFPOOL_START_TIMEOUT_SECONDS}s"
fi

surfpool_verify_rpc
surfpool_write_session "${pid}" "${effective_airdrop_lamports}" "${resumed_persistent_database}"
surfpool_pid_matches "${pid}" || surfpool_die "Surfpool process ownership verification failed"
surfpool_verify_runtime_env
ready=1
trap - EXIT INT TERM

surfpool_note "Surfpool ${SURFPOOL_EXPECTED_VERSION} is ready (pid ${pid})"
surfpool_note "RPC: ${SURFPOOL_RPC_URL}"
surfpool_note "Application environment: ${SURFPOOL_RUNTIME_ENV}"

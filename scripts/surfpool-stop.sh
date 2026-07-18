#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib/surfpool-common.sh
source "${SCRIPT_DIR}/lib/surfpool-common.sh"

surfpool_require_harness_commands
surfpool_validate_settings
surfpool_resolve_binary

pid="$(surfpool_read_pid || true)"
if [[ -z "${pid}" ]]; then
  if surfpool_port_is_listening "${SURFPOOL_PORT}"; then
    surfpool_die "RPC port ${SURFPOOL_PORT} is in use, but this harness has no PID file; refusing to stop it"
  fi
  surfpool_note "Surfpool is not running"
  exit 0
fi

if ! surfpool_pid_is_live "${pid}"; then
  surfpool_warn "removing stale PID file for ${pid}"
  rm -f "${SURFPOOL_PID_FILE}"
  exit 0
fi

surfpool_pid_matches "${pid}" ||
  surfpool_die "PID ${pid} does not match the Surfpool process started by this harness"

kill -TERM "${pid}"
deadline=$((SECONDS + SURFPOOL_STOP_TIMEOUT_SECONDS))
while surfpool_pid_is_live "${pid}" && ((SECONDS < deadline)); do
  sleep 0.25
done

if surfpool_pid_is_live "${pid}"; then
  surfpool_pid_matches "${pid}" ||
    surfpool_die "PID ${pid} changed identity while stopping; refusing SIGKILL"
  surfpool_warn "Surfpool did not stop after ${SURFPOOL_STOP_TIMEOUT_SECONDS}s; sending SIGKILL"
  kill -KILL "${pid}"
fi

rm -f "${SURFPOOL_PID_FILE}"
surfpool_note "Surfpool stopped (pid ${pid})"

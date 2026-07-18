#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib/surfpool-common.sh
source "${SCRIPT_DIR}/lib/surfpool-common.sh"

live=0
if [[ "${1:-}" == "--live" ]]; then
  live=1
elif [[ $# -ne 0 ]]; then
  surfpool_die "usage: $0 [--live]"
fi

for script in "${SCRIPT_DIR}"/*.sh "${SCRIPT_DIR}"/lib/*.sh; do
  bash -n "${script}"
done
bash -n "${PROJECT_ROOT}/configs/surfpool.env"

surfpool_require_harness_commands
surfpool_validate_settings
surfpool_verify_install

if (SURFPOOL_HOST=0.0.0.0; surfpool_validate_settings) >/dev/null 2>&1; then
  surfpool_die "non-loopback host self-check failed"
fi
if (SURFPOOL_NETWORK=devnet; surfpool_validate_settings) >/dev/null 2>&1; then
  surfpool_die "non-mainnet datasource self-check failed"
fi

payload="$(jq -cn '{jsonrpc:"2.0",id:1,method:"getVersion",params:[]}')"
jq -e '.method == "getVersion" and .params == []' >/dev/null <<<"${payload}" ||
  surfpool_die "JSON-RPC payload self-check failed"

if ((live)); then
  started_here=0
  existing_pid="$(surfpool_read_pid || true)"
  if [[ -z "${existing_pid}" ]]; then
    started_here=1
    trap 'if ((started_here)); then "${SCRIPT_DIR}/surfpool-stop.sh" >/dev/null 2>&1 || true; fi' EXIT
    "${SCRIPT_DIR}/surfpool-start.sh"
  fi
  "${SCRIPT_DIR}/surfpool-doctor.sh"
  if ((started_here)); then
    "${SCRIPT_DIR}/surfpool-stop.sh"
    started_here=0
  fi
fi

surfpool_note "Surfpool harness self-check: passed"

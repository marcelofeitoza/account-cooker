#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib/surfpool-common.sh
source "${SCRIPT_DIR}/lib/surfpool-common.sh"

surfpool_require_harness_commands
surfpool_require_command cargo
surfpool_validate_settings
surfpool_verify_install

pid="$(surfpool_read_pid || true)"
[[ -n "${pid}" ]] || surfpool_die "Surfpool PID file is missing"
surfpool_pid_matches "${pid}" || surfpool_die "Surfpool PID ${pid} is not live or does not match"

[[ -r "${SURFPOOL_KEYPAIR}" ]] || surfpool_die "local funder key is missing"
[[ "$(surfpool_file_mode "${SURFPOOL_KEYPAIR}")" == "600" ]] ||
  surfpool_die "local funder key must have mode 600"
cargo run \
  --quiet \
  --locked \
  --manifest-path "${PROJECT_ROOT}/Cargo.toml" \
  -p account-cooker \
  -- keygen \
  --project-root "${PROJECT_ROOT}" \
  --output "${SURFPOOL_KEYPAIR}" \
  >/dev/null || surfpool_die "local funder keypair is invalid"
[[ -e "${SURFPOOL_DB}" ]] || surfpool_die "persistent Surfpool database is missing"

surfpool_verify_runtime_env
surfpool_verify_rpc

session_pid="$(jq -er '.pid | numbers' "${SURFPOOL_SESSION_FILE}")" ||
  surfpool_die "Surfpool session metadata is missing or invalid"
[[ "${session_pid}" == "${pid}" ]] || surfpool_die "session metadata PID does not match"
jq -e \
  --arg hash "${SURFPOOL_EXPECTED_SHA256}" \
  --arg version "${SURFPOOL_EXPECTED_VERSION}" \
  --arg rpc "${SURFPOOL_RPC_URL}" \
  '.binarySha256 == $hash and .surfpoolVersion == $version and .rpcUrl == $rpc and .network == "mainnet"' \
  "${SURFPOOL_SESSION_FILE}" >/dev/null || surfpool_die "session metadata does not match the pinned environment"

surfpool_note "Surfpool doctor: healthy"
surfpool_note "  pid: ${pid}"
surfpool_note "  version: ${SURFPOOL_EXPECTED_VERSION}"
surfpool_note "  sha256: ${SURFPOOL_EXPECTED_SHA256}"
surfpool_note "  rpc: ${SURFPOOL_RPC_URL}"
surfpool_note "  surfnet: ${SURFPOOL_ID}"

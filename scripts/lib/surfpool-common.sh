#!/usr/bin/env bash

# Shared implementation for the Surfpool lifecycle scripts. This file is meant
# to be sourced, not executed.

if [[ -n "${SURFPOOL_COMMON_LOADED:-}" ]]; then
  return 0
fi
SURFPOOL_COMMON_LOADED=1

SURFPOOL_LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SURFPOOL_LIB_DIR}/../.." && pwd)"
SURFPOOL_CONFIG_FILE="${SURFPOOL_CONFIG_FILE:-${PROJECT_ROOT}/configs/surfpool.env}"

if [[ ! -r "${SURFPOOL_CONFIG_FILE}" ]]; then
  printf 'error: Surfpool config is not readable: %s\n' "${SURFPOOL_CONFIG_FILE}" >&2
  exit 1
fi

# shellcheck source=../../configs/surfpool.env
source "${SURFPOOL_CONFIG_FILE}"

case "$(uname -s):$(uname -m)" in
  Darwin:arm64)
    SURFPOOL_EXPECTED_SHA256="${SURFPOOL_EXPECTED_SHA256_DARWIN_ARM64}"
    ;;
  Linux:x86_64)
    SURFPOOL_EXPECTED_SHA256="${SURFPOOL_EXPECTED_SHA256_LINUX_X64}"
    ;;
  *)
    printf 'error: no pinned Surfpool binary is configured for %s:%s\n' \
      "$(uname -s)" "$(uname -m)" >&2
    exit 1
    ;;
esac
readonly SURFPOOL_EXPECTED_SHA256

SURFPOOL_HOME="${SURFPOOL_HOME:-${PROJECT_ROOT}/.surfpool}"
SURFPOOL_KEYS_DIR="${SURFPOOL_KEYS_DIR:-${SURFPOOL_HOME}/keys}"
SURFPOOL_STATE_DIR="${SURFPOOL_STATE_DIR:-${SURFPOOL_HOME}/state}"
SURFPOOL_LOG_DIR="${SURFPOOL_LOG_DIR:-${SURFPOOL_HOME}/logs}"
SURFPOOL_KEYPAIR="${SURFPOOL_KEYPAIR:-${SURFPOOL_KEYS_DIR}/funder.json}"
SURFPOOL_DB="${SURFPOOL_DB:-${SURFPOOL_STATE_DIR}/noise.sqlite}"
SURFPOOL_PID_FILE="${SURFPOOL_PID_FILE:-${SURFPOOL_HOME}/surfpool.pid}"
SURFPOOL_SESSION_FILE="${SURFPOOL_SESSION_FILE:-${SURFPOOL_HOME}/session.json}"
SURFPOOL_RUNTIME_ENV="${SURFPOOL_RUNTIME_ENV:-${SURFPOOL_HOME}/runtime.env}"
SURFPOOL_LAUNCHER_LOG="${SURFPOOL_LAUNCHER_LOG:-${SURFPOOL_LOG_DIR}/launcher.log}"
SURFPOOL_SNAPSHOT_ARCHIVE="${PROJECT_ROOT}/${SURFPOOL_SNAPSHOT_ARCHIVE_RELATIVE}"
SURFPOOL_SNAPSHOT_FILE="${SURFPOOL_STATE_DIR}/jupiter-reviewed.snapshot.json"
SURFPOOL_RPC_URL="http://${SURFPOOL_HOST}:${SURFPOOL_PORT}"
SURFPOOL_WS_URL="ws://${SURFPOOL_HOST}:${SURFPOOL_WS_PORT}"

surfpool_note() {
  printf '%s\n' "$*"
}

surfpool_warn() {
  printf 'warning: %s\n' "$*" >&2
}

surfpool_die() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

surfpool_require_command() {
  command -v "$1" >/dev/null 2>&1 || surfpool_die "required command not found: $1"
}

surfpool_is_uint() {
  [[ "$1" =~ ^[0-9]+$ ]]
}

surfpool_validate_settings() {
  [[ "${SURFPOOL_NETWORK}" == "mainnet" ]] ||
    surfpool_die "SURFPOOL_NETWORK must be mainnet for the canonical lazy fork"
  [[ "${SURFPOOL_HOST}" == "127.0.0.1" ]] ||
    surfpool_die "SURFPOOL_HOST must be the explicit loopback address 127.0.0.1"

  surfpool_is_uint "${SURFPOOL_PORT}" || surfpool_die "invalid Surfpool RPC port"
  surfpool_is_uint "${SURFPOOL_WS_PORT}" || surfpool_die "invalid Surfpool WebSocket port"
  surfpool_is_uint "${SURFPOOL_STUDIO_PORT}" || surfpool_die "invalid Surfpool Studio port"
  ((SURFPOOL_PORT > 0 && SURFPOOL_PORT <= 65535)) || surfpool_die "invalid Surfpool RPC port"
  ((SURFPOOL_WS_PORT > 0 && SURFPOOL_WS_PORT <= 65535)) ||
    surfpool_die "invalid Surfpool WebSocket port"
  ((SURFPOOL_STUDIO_PORT > 0 && SURFPOOL_STUDIO_PORT <= 65535)) ||
    surfpool_die "invalid Surfpool Studio port"
  [[ "${SURFPOOL_PORT}" != "${SURFPOOL_WS_PORT}" && \
    "${SURFPOOL_PORT}" != "${SURFPOOL_STUDIO_PORT}" && \
    "${SURFPOOL_WS_PORT}" != "${SURFPOOL_STUDIO_PORT}" ]] ||
    surfpool_die "Surfpool RPC, WebSocket, and Studio ports must differ"

  surfpool_is_uint "${SURFPOOL_START_TIMEOUT_SECONDS}" ||
    surfpool_die "invalid Surfpool startup timeout"
  surfpool_is_uint "${SURFPOOL_STOP_TIMEOUT_SECONDS}" ||
    surfpool_die "invalid Surfpool stop timeout"
  surfpool_is_uint "${SURFPOOL_AIRDROP_LAMPORTS}" ||
    surfpool_die "invalid Surfpool airdrop amount"
  surfpool_is_uint "${SURFPOOL_MAX_PROFILES}" ||
    surfpool_die "invalid Surfpool profile limit"
  ((SURFPOOL_MAX_PROFILES >= 1)) || surfpool_die "Surfpool profile limit must be positive"
}

surfpool_resolve_binary() {
  if [[ -n "${SURFPOOL_BIN:-}" ]]; then
    [[ -x "${SURFPOOL_BIN}" ]] || surfpool_die "SURFPOOL_BIN is not executable: ${SURFPOOL_BIN}"
  else
    SURFPOOL_BIN="$(command -v surfpool 2>/dev/null || true)"
    [[ -n "${SURFPOOL_BIN}" ]] || surfpool_die "surfpool is not on PATH"
  fi

  case "${SURFPOOL_BIN}" in
    /*) ;;
    *) SURFPOOL_BIN="$(cd "$(dirname "${SURFPOOL_BIN}")" && pwd)/$(basename "${SURFPOOL_BIN}")" ;;
  esac
}

surfpool_file_sha256() {
  local file="$1"

  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "${file}" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "${file}" | awk '{print $1}'
  else
    surfpool_die "neither sha256sum nor shasum is available"
  fi
}

surfpool_verify_install() {
  local raw_version actual_version actual_sha256

  surfpool_resolve_binary
  raw_version="$("${SURFPOOL_BIN}" --version 2>/dev/null)" ||
    surfpool_die "failed to execute ${SURFPOOL_BIN} --version"
  actual_version="${raw_version#surfpool }"
  [[ "${actual_version}" == "${SURFPOOL_EXPECTED_VERSION}" ]] ||
    surfpool_die "Surfpool version mismatch: expected ${SURFPOOL_EXPECTED_VERSION}, got ${raw_version}"

  actual_sha256="$(surfpool_file_sha256 "${SURFPOOL_BIN}")"
  [[ "${actual_sha256}" == "${SURFPOOL_EXPECTED_SHA256}" ]] ||
    surfpool_die "Surfpool binary hash mismatch: expected ${SURFPOOL_EXPECTED_SHA256}, got ${actual_sha256}"
}

surfpool_prepare_snapshot() {
  local archive_sha256 snapshot_sha256 temp_file

  [[ -r "${SURFPOOL_SNAPSHOT_ARCHIVE}" ]] ||
    surfpool_die "pinned Surfpool snapshot archive is missing"
  archive_sha256="$(surfpool_file_sha256 "${SURFPOOL_SNAPSHOT_ARCHIVE}")"
  [[ "${archive_sha256}" == "${SURFPOOL_SNAPSHOT_ARCHIVE_SHA256}" ]] ||
    surfpool_die "pinned Surfpool snapshot archive hash mismatch"

  if [[ -r "${SURFPOOL_SNAPSHOT_FILE}" ]]; then
    snapshot_sha256="$(surfpool_file_sha256 "${SURFPOOL_SNAPSHOT_FILE}")"
    if [[ "${snapshot_sha256}" == "${SURFPOOL_SNAPSHOT_SHA256}" ]]; then
      return 0
    fi
  fi

  temp_file="${SURFPOOL_SNAPSHOT_FILE}.tmp.$$"
  gzip -cd "${SURFPOOL_SNAPSHOT_ARCHIVE}" >"${temp_file}" || {
    rm -f "${temp_file}"
    surfpool_die "failed to decompress the pinned Surfpool snapshot"
  }
  snapshot_sha256="$(surfpool_file_sha256 "${temp_file}")"
  if [[ "${snapshot_sha256}" != "${SURFPOOL_SNAPSHOT_SHA256}" ]]; then
    rm -f "${temp_file}"
    surfpool_die "decompressed Surfpool snapshot hash mismatch"
  fi
  chmod 600 "${temp_file}"
  mv "${temp_file}" "${SURFPOOL_SNAPSHOT_FILE}"
}

surfpool_rpc_call() {
  local method="$1"
  local params_json="${2:-[]}"
  local payload

  payload="$(jq -cn --arg method "${method}" --argjson params "${params_json}" \
    '{jsonrpc:"2.0",id:1,method:$method,params:$params}')" ||
    surfpool_die "invalid JSON-RPC params for ${method}"

  curl --silent --show-error --max-time 3 \
    --header 'Content-Type: application/json' \
    --data "${payload}" \
    "${SURFPOOL_RPC_URL}"
}

surfpool_rpc_is_ready() {
  local response

  response="$(surfpool_rpc_call getVersion '[]' 2>/dev/null)" || return 1
  jq -e '.error == null and (.result["surfnet-version"] | type == "string")' \
    >/dev/null 2>&1 <<<"${response}"
}

surfpool_verify_rpc() {
  local version_response info_response rpc_version

  version_response="$(surfpool_rpc_call getVersion '[]')" ||
    surfpool_die "Surfpool JSON-RPC is unavailable at ${SURFPOOL_RPC_URL}"
  rpc_version="$(jq -er '.result["surfnet-version"] | strings' <<<"${version_response}")" ||
    surfpool_die "getVersion did not identify a Surfpool RPC"
  [[ "${rpc_version}" == "${SURFPOOL_EXPECTED_VERSION}" ]] ||
    surfpool_die "Surfpool RPC version mismatch: expected ${SURFPOOL_EXPECTED_VERSION}, got ${rpc_version}"

  info_response="$(surfpool_rpc_call surfnet_getSurfnetInfo '[]')" ||
    surfpool_die "surfnet_getSurfnetInfo failed"
  jq -e '.error == null and (.result.value | type == "object")' >/dev/null <<<"${info_response}" ||
    surfpool_die "surfnet_getSurfnetInfo returned an invalid response"
}

surfpool_wait_for_rpc() {
  local pid="$1"
  local deadline=$((SECONDS + SURFPOOL_START_TIMEOUT_SECONDS))

  while ((SECONDS < deadline)); do
    kill -0 "${pid}" 2>/dev/null || return 1
    surfpool_rpc_is_ready && return 0
    sleep 0.25
  done
  return 1
}

surfpool_read_pid() {
  local pid

  [[ -r "${SURFPOOL_PID_FILE}" ]] || return 1
  IFS= read -r pid <"${SURFPOOL_PID_FILE}"
  surfpool_is_uint "${pid}" || return 1
  printf '%s\n' "${pid}"
}

surfpool_pid_is_live() {
  kill -0 "$1" 2>/dev/null
}

surfpool_pid_matches() {
  local pid="$1"
  local command_line

  surfpool_pid_is_live "${pid}" || return 1
  command_line="$(ps -p "${pid}" -o command= 2>/dev/null)" || return 1
  [[ "${command_line}" == "${SURFPOOL_BIN} start "* || "${command_line}" == "${SURFPOOL_BIN} start" ]]
}

surfpool_port_is_listening() {
  lsof -nP -iTCP:"$1" -sTCP:LISTEN -t 2>/dev/null | awk 'NF { found=1 } END { exit !found }'
}

surfpool_check_ports_free() {
  surfpool_port_is_listening "${SURFPOOL_PORT}" &&
    surfpool_die "RPC port ${SURFPOOL_PORT} already has a listener"
  surfpool_port_is_listening "${SURFPOOL_WS_PORT}" &&
    surfpool_die "WebSocket port ${SURFPOOL_WS_PORT} already has a listener"
  surfpool_port_is_listening "${SURFPOOL_STUDIO_PORT}" &&
    surfpool_die "Studio port ${SURFPOOL_STUDIO_PORT} already has a listener"
  return 0
}

surfpool_file_mode() {
  if stat -f '%Lp' "$1" >/dev/null 2>&1; then
    stat -f '%Lp' "$1"
  else
    stat -c '%a' "$1"
  fi
}

surfpool_write_runtime_env() {
  local temp_file="${SURFPOOL_RUNTIME_ENV}.tmp.$$"

  umask 077
  {
    printf 'COOKER_NETWORK=surfpool\n'
    printf 'COOKER_REQUIRE_SURFPOOL=true\n'
    printf 'COOKER_RPC_URL=%s\n' "${SURFPOOL_RPC_URL}"
    printf 'COOKER_WS_URL=%s\n' "${SURFPOOL_WS_URL}"
    printf 'COOKER_SIGNER_PATH=%s\n' "${SURFPOOL_KEYPAIR}"
  } >"${temp_file}"
  chmod 600 "${temp_file}"
  mv "${temp_file}" "${SURFPOOL_RUNTIME_ENV}"
}

surfpool_verify_runtime_env() {
  local key value
  local found_network=0 found_required=0 found_rpc=0 found_ws=0 found_signer=0

  [[ -r "${SURFPOOL_RUNTIME_ENV}" ]] ||
    surfpool_die "runtime environment is missing: ${SURFPOOL_RUNTIME_ENV}"

  while IFS='=' read -r key value; do
    case "${key}" in
      COOKER_NETWORK)
        [[ "${value}" == "surfpool" ]] || surfpool_die "invalid COOKER_NETWORK"
        found_network=1
        ;;
      COOKER_REQUIRE_SURFPOOL)
        [[ "${value}" == "true" ]] || surfpool_die "COOKER_REQUIRE_SURFPOOL must be true"
        found_required=1
        ;;
      COOKER_RPC_URL)
        [[ "${value}" == "${SURFPOOL_RPC_URL}" ]] || surfpool_die "application RPC is not canonical"
        found_rpc=1
        ;;
      COOKER_WS_URL)
        [[ "${value}" == "${SURFPOOL_WS_URL}" ]] || surfpool_die "application WebSocket is not canonical"
        found_ws=1
        ;;
      COOKER_SIGNER_PATH)
        [[ "${value}" == "${SURFPOOL_KEYPAIR}" ]] || surfpool_die "application signer path is not canonical"
        found_signer=1
        ;;
      '') ;;
      *) surfpool_die "unexpected runtime environment key: ${key}" ;;
    esac
  done <"${SURFPOOL_RUNTIME_ENV}"

  ((found_network && found_required && found_rpc && found_ws && found_signer)) ||
    surfpool_die "runtime environment is incomplete"
}

surfpool_write_session() {
  local pid="$1"
  local effective_airdrop_lamports="$2"
  local resumed_persistent_database="$3"
  local binary_sha256 started_at temp_file

  binary_sha256="$(surfpool_file_sha256 "${SURFPOOL_BIN}")"
  started_at="$(date -u '+%Y-%m-%dT%H:%M:%SZ')"
  temp_file="${SURFPOOL_SESSION_FILE}.tmp.$$"
  jq -n \
    --arg startedAt "${started_at}" \
    --arg binary "${SURFPOOL_BIN}" \
    --arg binarySha256 "${binary_sha256}" \
    --arg surfpoolVersion "${SURFPOOL_EXPECTED_VERSION}" \
    --arg network "${SURFPOOL_NETWORK}" \
    --arg surfnetId "${SURFPOOL_ID}" \
    --arg rpcUrl "${SURFPOOL_RPC_URL}" \
    --arg wsUrl "${SURFPOOL_WS_URL}" \
    --arg database "${SURFPOOL_DB}" \
    --arg snapshotArchiveSha256 "${SURFPOOL_SNAPSHOT_ARCHIVE_SHA256}" \
    --arg snapshotSha256 "${SURFPOOL_SNAPSHOT_SHA256}" \
    --argjson configuredAirdropLamports "${SURFPOOL_AIRDROP_LAMPORTS}" \
    --argjson effectiveAirdropLamports "${effective_airdrop_lamports}" \
    --argjson resumedPersistentDatabase "${resumed_persistent_database}" \
    --argjson pid "${pid}" \
    '{startedAt:$startedAt,pid:$pid,binary:$binary,binarySha256:$binarySha256,surfpoolVersion:$surfpoolVersion,network:$network,surfnetId:$surfnetId,rpcUrl:$rpcUrl,wsUrl:$wsUrl,database:$database,snapshotArchiveSha256:$snapshotArchiveSha256,snapshotSha256:$snapshotSha256,configuredAirdropLamports:$configuredAirdropLamports,effectiveAirdropLamports:$effectiveAirdropLamports,resumedPersistentDatabase:$resumedPersistentDatabase}' \
    >"${temp_file}"
  chmod 600 "${temp_file}"
  mv "${temp_file}" "${SURFPOOL_SESSION_FILE}"
}

surfpool_require_harness_commands() {
  surfpool_require_command awk
  surfpool_require_command curl
  surfpool_require_command gzip
  surfpool_require_command jq
  surfpool_require_command lsof
  surfpool_require_command ps
}

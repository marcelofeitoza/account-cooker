#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

if (($# != 0)); then
  printf 'usage: %s\n' "$0" >&2
  exit 1
fi

for command in cp find git gitleaks mkdir mktemp tar; do
  command -v "${command}" >/dev/null 2>&1 || {
    printf 'error: required command not found: %s\n' "${command}" >&2
    exit 1
  }
done

temp_root="${TMPDIR:-/tmp}"
snapshot="$(mktemp -d "${temp_root%/}/account-cooker-gitleaks.XXXXXX")"
paths_file="${snapshot}.paths"
cleanup() {
  local exit_status=$?
  trap - EXIT INT TERM
  if [[ -d "${snapshot}" ]]; then
    find "${snapshot}" -depth -delete
  fi
  if [[ -f "${paths_file}" ]]; then
    find "${paths_file}" -delete
  fi
  exit "${exit_status}"
}
trap cleanup EXIT INT TERM

cd "${PROJECT_ROOT}"
git archive --format=tar HEAD | tar -xf - -C "${snapshot}"
git diff --no-renames --name-only -z HEAD -- >"${paths_file}"
git ls-files -z --others --exclude-standard >>"${paths_file}"

while IFS= read -r -d '' relative; do
  case "${relative}" in
    '' | /* | .. | ../* | */.. | */../*)
      printf 'error: Git returned an unsafe worktree path\n' >&2
      exit 1
      ;;
  esac

  destination="${snapshot}/${relative}"
  if [[ -e "${destination}" || -L "${destination}" ]]; then
    find "${destination}" -depth -delete
  fi
  if [[ -f "./${relative}" || -L "./${relative}" ]]; then
    mkdir -p "$(dirname "${destination}")"
    cp -pP "./${relative}" "${destination}"
  fi
done <"${paths_file}"

(
  cd "${snapshot}"
  gitleaks dir --redact --no-banner --verbose \
    --config "${PROJECT_ROOT}/.gitleaks.toml" .
)

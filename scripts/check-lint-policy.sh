#!/usr/bin/env bash
# Verify the declared safety and advisory policy is present where a reviewer
# reads it: an explicit `#![forbid(unsafe_code)]` in every crate root, the
# workspace lint that backs it, and a checked-in RustSec policy that denies
# advisory warnings rather than only printing them.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

if (($# != 0)); then
  printf 'usage: %s\n' "$0" >&2
  exit 1
fi

cd "${PROJECT_ROOT}"

failures=0
fail() {
  printf 'error: %s\n' "$1" >&2
  failures=$((failures + 1))
}

members=()
while IFS= read -r member; do
  members+=("${member}")
done < <(sed -n '/^\[workspace\]/,/^\[[^w]/p' Cargo.toml |
  sed -n 's/^ *"\(crates\/[a-z0-9-]*\)",$/\1/p')

if ((${#members[@]} == 0)); then
  fail 'no workspace members were parsed from Cargo.toml'
fi

on_disk=0
for manifest in crates/*/Cargo.toml; do
  if [[ -f "${manifest}" ]]; then
    on_disk=$((on_disk + 1))
  fi
done
if ((${#members[@]} != on_disk)); then
  fail "parsed ${#members[@]} workspace member(s) but found ${on_disk} crate manifest(s)"
fi

roots_checked=0
for member in "${members[@]}"; do
  manifest="${member}/Cargo.toml"
  if [[ ! -f "${manifest}" ]]; then
    fail "workspace member has no manifest: ${manifest}"
    continue
  fi

  if ! grep -qx 'workspace = true' "${manifest}"; then
    fail "${manifest} does not inherit the workspace lint table"
  fi

  found_root=0
  for root in "${member}/src/lib.rs" "${member}/src/main.rs"; do
    [[ -f "${root}" ]] || continue
    found_root=1
    roots_checked=$((roots_checked + 1))
    if ! grep -qxF '#![forbid(unsafe_code)]' "${root}"; then
      fail "${root} is missing an explicit #![forbid(unsafe_code)]"
    fi
  done

  if ((found_root == 0)); then
    fail "workspace member has no src/lib.rs or src/main.rs: ${member}"
  fi
done

if ! grep -qxF 'unsafe_code = "forbid"' Cargo.toml; then
  fail 'the workspace [lints.rust] table does not forbid unsafe_code'
fi

audit_policy='.cargo/audit.toml'
if [[ ! -f "${audit_policy}" ]]; then
  fail "the RustSec advisory policy is missing: ${audit_policy}"
else
  if ! grep -qx '\[advisories\]' "${audit_policy}"; then
    fail "${audit_policy} has no [advisories] section"
  fi
  for level in warnings unmaintained unsound yanked; do
    if ! grep -q "^deny = .*\"${level}\"" "${audit_policy}"; then
      fail "${audit_policy} does not deny ${level}"
    fi
  done
fi

if ((failures != 0)); then
  printf 'lint policy check failed with %d problem(s)\n' "${failures}" >&2
  exit 1
fi

printf 'lint policy ok: %d crate root(s) forbid unsafe code, %s denies advisory warnings\n' \
  "${roots_checked}" "${audit_policy}"

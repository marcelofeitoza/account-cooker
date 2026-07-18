# Evidence

`evidence/final` is generated only by a canonical `scripts/full-demo.sh` run from a clean
Git commit. Reduced rehearsals go to ignored `evidence/tmp` paths and cannot be promoted or
used to claim the canonical scale gates.

Status: no canonical `evidence/final` pack is present yet. The canonical run and both
fresh-clone repetitions remain pending.

## Generate

```bash
./scripts/full-demo.sh
```

The command validates the raw run and atomically creates `evidence/final`. It refuses a
dirty source tree, an existing destination, wrong soak dimensions, failed recovery or
postconditions, incomplete evaluator rows, mismatched source commit, malformed Surfpool
session provenance, or unsanitized signature samples.

## Contents

- `manifest.json`: source, environment, gate, total, and safety summary.
- `run.json`, `executions.json`: sanitized invocation and per-command provenance.
- `quality-gates.json`: format, lint, test, shell, dependency, secret, and release-build gates.
- `cli-workflow.json`: read-only doctor/previews, funding, execution, status, and recovery.
- `transactions.json`: six acceptance groups with shortened local signatures, including
  all six real `SIGKILL` process-recovery cases.
- `virtual-soak.json`: five-seed, 1,000-agent by 30-day deterministic replay proof.
- `surfpool-soak.json`: faulted 1,000-transaction restart/reconciliation proof.
- `surfpool-provenance.json`: pinned binary, snapshot, fresh-node, database-resume, and
  airdrop provenance.
- `metrics.json`, `metrics.csv`, `metrics.md`: all evaluator seeds, ablations, aggregates,
  and limitations.
- `config.executed.toml`, `reproduce.txt`, `report.md`, and `test-summary.txt`: reviewer
  entry points. The executed configuration contains only loopback URLs and public policy
  values; signer paths and secrets are excluded.
- `checksums.txt`: SHA-256 for every other file in the pack.

Verify the pack from this directory with either platform command:

```bash
sha256sum --check checksums.txt
# or
shasum -a 256 --check checksums.txt
```

## Excluded Material

The committed pack contains no generated keypair, signed transaction bytes, full local
signature, mutable database, raw RPC/program log, absolute workstation path, credential,
or decompressed account snapshot. Those artifacts stay in ignored `.surfpool` and
`evidence/raw` directories and are not needed to reproduce the proof.

## Interpretation

The evidence can establish only its named engineering invariants and synthetic evaluator
metrics. It does not establish anonymity or resemblance to all human Solana activity. In
particular:

- a common funding graph remains directly observable;
- evaluator attacks and ownership labels use synthetic known ground truth;
- reviewed Jupiter state is a 21-account offline snapshot captured from a lazy fork and
  frozen at slot `433717382`;
- execution uses one local controller, SQLite store, local signers, and loopback Surfpool;
- the stateful adapter is native Solana stake, not Marinade.

Sponsor confirmation of AI-assistance eligibility under the listing's `HUMAN_ONLY` flag
is also outside this evidence pack and remains required before submission.

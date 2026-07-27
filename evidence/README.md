# Evidence

`evidence/final` is generated only by a canonical `scripts/full-demo.sh` run from a clean
Git commit. Reduced rehearsals go to ignored `evidence/tmp` paths and cannot be promoted or
used to claim the canonical scale gates.

Status: the canonical `evidence/final` pack and both fresh-clone repetitions passed on
2026-07-18 from source commit `8cc338e968fb2ff561508ba2ca69d113e20035b9`.

`evidence/devnet` is a separate, smaller record from one bounded public Solana devnet run.
It is produced by the opt-in `scripts/devnet-soak.sh` and is never part of the canonical
pack, because the canonical pack is defined to contain no public-network transaction. The
two records are kept side by side deliberately; see
[the devnet run and topology delta](../docs/DEVNET.md) for what the public run establishes
and leaves unmeasured.

## Generate

```bash
./scripts/full-demo.sh                                # evidence/final
./scripts/devnet-soak.sh --transactions 200 --promote # evidence/devnet
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
- `../clean-clone-verification.json`: sanitized hashes and invariant results for the
  primary canonical run and two independent clean-clone repetitions.
- `../devnet/devnet-soak.json` and `../devnet/README.md`: the separate bounded public devnet
  record, with full explorer-checkable signatures and its own checksum file.

Verify the pack from this directory with either platform command:

```bash
sha256sum --check checksums.txt
# or
shasum -a 256 --check checksums.txt
```

## Excluded Material

The committed pack contains no generated keypair, signed transaction bytes, full local
signature, mutable database, raw RPC/program log, absolute workstation path, credential,
or decompressed account snapshot. Those artifacts stay in ignored `.surfpool`, `.devnet`,
and `evidence/raw` directories and are not needed to reproduce the proof.

`evidence/devnet` deliberately commits full transaction signatures. A local Surfpool
signature means nothing outside the surfnet that produced it, so shortening it costs a
reviewer nothing. A devnet signature is public record and is the direct identifier needed
to query and verify each recorded transaction. No key, no signed transaction bytes, and no
database leave the ignored directories in either case.

## Interpretation

The evidence can establish only its named engineering invariants and synthetic evaluator
metrics. It does not establish anonymity or resemblance to all human Solana activity. In
particular:

- a common funding graph remains directly observable;
- evaluator attacks and ownership labels use synthetic known ground truth;
- reviewed Jupiter state is a 21-account offline snapshot captured from a lazy fork and
  frozen at slot `433717382`;
- execution in this pack uses one local controller, SQLite store, local signers, and
  loopback Surfpool; the separate `evidence/devnet` record covers one bounded public devnet
  run and is not a sustained-load result;
- the stateful adapter is native Solana stake, not Marinade.

The separate `docs/ELIGIBILITY.md` audit covers the human submission path and disclosed
AI assistance. Personal eligibility attestations, the completed manual submission, and any
KYC or payout requirements remain outside this evidence pack and under Marcelo's control.

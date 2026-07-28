# Funding-Provenance Measurement

This record answers one question: does the funding graph still identify the operator after
the funding path changes, and by how much?

Before this work the evaluator scored the common-funder attack at ROC AUC 1.0000 for every
planner and every seed, and the repository said so. That number was correct and it was the
sharpest remaining limit on the whole result: the behavioral engine hid behavior while a
single funding edge re-linked the fleet for free.

## Scope Of The Claim

This is an evaluator result. `cooker-core::funding` builds the schedules and
`cooker-eval` measures them; the runtime `cooker fund` command is unchanged and still pays
each account from the operator wallet. Nothing here is a deployed pool, a custodial
service, or an on-chain construction. The result says what a payer assignment does to a
named attack under a named observation model. It does not say that such a pool exists, that
it would be safe to operate, or that operating one is legal in any jurisdiction.

## What Was Run

```bash
cooker init --output config.toml
cooker evaluate --config config.toml --output-dir evidence-run
```

Toolchain `1.92.0`, exactly as pinned in `rust-toolchain.toml`. No network, no signer, no
chain. The evaluator is byte-deterministic, which `experiment_is_byte_deterministic`
enforces and which two independent runs confirmed by producing identical checksums.

Settings: 10 controllers, 10 accounts each, 30 virtual days, 2 events per account per day,
held-out seeds `[11, 23, 37, 51, 71]`, fixed threshold `0.550`. Three funding schemes times
three planners times five seeds times six ablations, 270 rows.

## Result

Mean funding ROC AUC across the five held-out seeds. The value is identical for all three
planners because the payer assignment does not read behavior, so only one row per scheme is
shown here; the per-planner and per-seed rows are in `evaluation.md`.

| Funding scheme | Common funder | Payer and round | Scheme-aware batch | Funding separation |
|---|---:|---:|---:|---:|
| `dedicated_per_operator` (baseline) | 1.0000 | 1.0000 | 1.0000 | 1.0000 |
| `pooled_mixed_rounds` (mitigation) | 0.5054 | 0.4977 | 0.4980 | 0.0028 |
| `pooled_per_operator_rounds` (control) | 0.7203 | 0.9487 | 0.9474 | 0.1607 |

Per-seed range for the mitigation, common-funder attack: min 0.4831, max 0.5171. The
interval straddles chance. It was not tuned to land there.

Composite attacker, full fixed weights, mean across seeds:

| Planner | Dedicated | Pooled mixed |
|---|---:|---:|
| `naive_uniform` | 1.0000 | 1.0000 |
| `independent_weighted` | 1.0000 | 0.7111 |
| `persona_session` | 0.9814 | 0.6751 |

## Why This Is Not An Artifact

The attacker was strengthened, never weakened. Two funding attacks were added and none was
removed: the payer-and-round overlap, and a scheme-aware attack that assumes the adversary
knows accounts are dealt into pooled batches and weights a shared batch by how few accounts
were in it. Both stay at 1.0000 on the baseline, so they are working at full strength.

Every other attacker channel is byte-identical across the three schemes. For seed 11 and
the persona planner: timing 0.4999, amount 0.4983, sequence 0.4747, destination 0.4977,
synchrony 0.4919, route 0.5000, balance rank 0.9089, in all three arms. A unit test asserts
this for every family, seed, planner, and ablation, so the funding movement cannot be a side
effect of perturbing behavior.

The control arm is what makes the claim causal. `pooled_per_operator_rounds` keeps the same
shared disburser set, the same uniform denomination, the same round structure, and the same
participation schedule, and only lets each operator's batch go to one disburser. It scores
0.9487 and 0.9474. A shared pool alone does not close the channel; mixing recipients across
operators inside each round is what closes it.

The mitigation cannot read the label it hides. Its schedule is a function of account
identifiers and a schedule seed only. `mixed_schedule_cannot_depend_on_operator_labels`
relabels every operator and requires the identical plan.

All three schemes replay one participation schedule derived from the master seed alone, so
which rounds an account is funded in is held constant and only the payer assignment differs.

## Residual And Cost

What the drop does not cover:

- Deposits into the pool. Where the disbursers get their balance is outside the observation
  model. An operator that funds the pool and then drains it through its own accounts can be
  re-linked by flow accounting that this evaluator does not perform. This is the largest
  remaining channel and it is why the limitation text still refuses the anonymity claim.
- Custody of the pool. A custodial disburser set is a trusted party. A non-custodial one
  needs its own construction and its own threat model.
- Batches of one. The smallest observed batch is 1 recipient in every scheme, which gives
  that transfer no cover at all. It leaks account-to-pool adjacency rather than
  account-to-operator adjacency, so it does not move the linkage number here, but it does
  combine with pool-deposit analysis.
- Fleet-level appearance. Every account is provisioned in round 0. A fleet that appears at
  once is a fleet-level signal that pairwise linkage does not score.

What it costs:

- Funding transfers. 400 for 100 accounts at 4 top-ups each, against 100 for a naive
  operator that provisions once. Four times the funding transactions, plus the operator's
  deposits into the pool, which are not counted here.
- Capital. A uniform denomination cannot match what an account actually needs, so the
  difference sits idle. The evaluator does not model account balances, so this is stated,
  not measured.
- Latency. An account waits for the next round rather than being funded on demand. Round
  length is a configured parameter, 24 hours in this run.

The transfer counts in `evaluation.md` are equal across the three schemes on purpose, so
the linkage comparison is not confounded by budget. They are not the cost comparison. The
cost comparison is against operator practice, stated above.

## Robustness

`the_mixed_pool_holds_across_funding_parameters` reruns the whole experiment at 2 disbursers
with 1 top-up per account and 24-hour rounds, and at 3 disbursers with 7 top-ups per account
and 12-hour rounds. All three funding attacks stay below 0.75 on the mitigation and the
baseline stays at exactly 1.0000 in both settings.

## Contents

- `evaluation.md`: the complete report, all 270 rows, every seed and ablation.
- `funding-summary.json`: config, per-scheme funding summary, per-seed schedule cost, and
  the limitation string.
- `checksums.txt`: SHA-256 of the two files above plus the two regenerable artifacts, so a
  reader can rerun the command and confirm byte equality rather than trusting this record.

`evidence/final` predates this measurement. Its funding rows are the pre-mitigation
baseline, which is now the `dedicated_per_operator` arm, and its evaluator artifacts carry
no funding-scheme column. The next canonical `scripts/full-demo.sh` run folds this
measurement into that pack.

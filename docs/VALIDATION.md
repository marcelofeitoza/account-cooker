# Validation And Evidence Plan

Status: acceptance contract; implementation has not started.

## 1. Quality Gates

The draft PR remains draft until these pass from a clean checkout:

    cargo fmt --all --check
    cargo clippy --workspace --all-targets --all-features -- -D warnings
    cargo test --workspace --all-features
    cargo test --workspace --doc
    ./scripts/surfpool-e2e.sh
    ./scripts/full-demo.sh

Supply-chain and benchmark gates are added after the core build if they do not weaken the
mandatory gates.

## 2. Test Layers

### Unit tests

- configuration validation;
- deterministic seed derivation;
- persona transition and duration sampling;
- amount bounds and roundness mixture;
- policy ordering and rejection;
- error classification;
- receipt parsing;
- feature extraction;
- metric sanity cases.

### Property tests

- probabilities normalize;
- generated timestamps are monotonic;
- no amount violates budget, reserve, or configured bounds;
- a fixed state and seed produce the same intent;
- ActionId is stable and collision-free over generated fixtures;
- state transitions reject illegal edges;
- a logical action has no more than one successful terminal outcome;
- scheduler concurrency stays within bounds;
- event serialization round-trips;
- evaluator labels cannot affect features.

### Store tests

- empty and incremental migrations;
- WAL initialization and integrity;
- unique action insertion;
- atomic lease claims;
- lease expiry;
- concurrent claim exclusion;
- submitted action reconciliation query;
- restart from each lifecycle state;
- immutable event ordering;
- run/network identity mismatch rejection.

### Scheduler tests

- 1,000-agent fairness;
- stable tie ordering;
- cancellation;
- backpressure;
- virtual-time advancement;
- fixed-seed trace hash;
- no one-task-per-wallet growth;
- skip days, sessions, cooldowns, and holds.

### Adapter tests

- mock only error/transport boundaries in unit tests;
- execute every successful chain path through Surfpool;
- assert both signature outcome and protocol-specific state changes;
- test deterministic, transient, unknown, and policy errors.

### Evaluator tests

- perfect-separation fixture produces expected high attacker score;
- random-label fixture remains near chance within tolerance;
- identical-trace fixture behaves predictably;
- threshold is selected without held-out labels;
- known clusters produce expected ARI/NMI;
- all seeds are reported;
- ablation changes only the intended feature family.

## 3. Surfpool Matrix

| Scenario | Required assertion |
|---|---|
| network guard | public and non-Surfpool RPCs fail before signer load |
| clean bootstrap | generated fleet and balances match manifest |
| native transfer | exact balance delta and confirmed receipt |
| SPL transfer | exact token delta and account ownership |
| Jupiter swap | bounded input/output delta and allowlisted route |
| stateful lifecycle | position/state exists and is observable |
| insufficient funds | no signature, policy/deterministic error |
| stale blockhash | safe refresh before send or classified expiry |
| simulation failure | no send attempted |
| response lost after send | one signature recovered |
| cooker crash | resume without duplicate logical action |
| Surfpool restart | reconcile against same persistent Surfnet |
| concurrent workers | one active wallet lease |
| budget exhaustion | no local transaction signature |
| kill switch | no new claims after activation |
| compressed soak | no unresolved Submitted action at finish |

## 4. Scale Runs

### Virtual run

- 1,000 agents;
- 30 virtual days;
- at least five master seeds;
- same agent definitions for baseline and modeled planner;
- deterministic output hash for each seed;
- bounded task count and streaming trace output;
- record wall time, peak memory when available, events, and database size.

### Surfpool run

- representative fleet subset for wall-clock operation;
- at least 1,000 locally executed transactions before final evidence when feasible;
- sampled Jupiter and stateful actions under strict local budgets;
- restart and fault injection during the run;
- zero duplicate logical intents;
- zero budget violations;
- zero unresolved Submitted actions after final reconciliation.

If the local transaction target is not reached, the exact achieved count and reason are
reported. Correctness is not weakened to manufacture a volume number.

## 5. Evaluation Runs

Compare:

- naive uniform planner;
- independent timing/weighted-action baseline;
- persona/session planner;
- one mitigation removed at a time.

Use at least five held-out seeds and equal:

- controller groups;
- agents;
- duration;
- action budget;
- asset budget;
- enabled adapters.

Report:

- per-feature distances;
- pairwise precision, recall, F1, ROC AUC;
- precision at K;
- clustering ARI or NMI;
- common-funder attack separately;
- mean, min, max, and per-seed values;
- action counts and rejected actions;
- failures and blind spots.

There is no predetermined requirement that the privacy hypothesis succeed. A result can
fail the claim gate while the runtime still passes its engineering gates.

## 6. Recovery Proof

The full demo injects crashes at:

1. after intent persistence;
2. after simulation;
3. after prepared transaction persistence;
4. after local signature creation;
5. after send response is lost;
6. after confirmation but before local promotion.

For each checkpoint:

- stop the cooker without cleanup;
- preserve the application database;
- restart against the same Surfnet;
- reconcile;
- assert one logical ActionId;
- assert zero or one landed signature as appropriate;
- assert final balances;
- record the event sequence.

## 7. Evidence Layout

Planned committed layout:

    evidence/
      README.md
      <run-id>/
        manifest.json
        report.md
        metrics.json
        metrics.csv
        transactions.json
        test-summary.txt
        config.redacted.toml
        checksums.txt

manifest.json includes:

- run ID and timestamp;
- Git revision and dirty-state flag;
- Rust and Cargo versions;
- Surfpool version and binary hash;
- Surfnet and snapshot/scenario identity;
- operating system and architecture;
- commands;
- config, model, fixture, and seed hashes;
- test results;
- local transaction signatures;
- metric artifact hashes;
- known limitations.

Raw logs, mutable databases, snapshots under review, and keys remain ignored. Only small,
redacted, reviewed artifacts are committed.

## 8. Secret And Provenance Audit

Before every push:

- inspect git status and staged diff;
- scan tracked files for keypair arrays, seed phrases, bearer tokens, RPC credentials,
  private URLs, and local databases;
- confirm no external private source, fixture, cached transaction corpus, or private
  documentation;
- confirm no competitor source was copied;
- record public dependencies and licenses;
- verify evidence uses generated aliases and local Surfpool signatures.

## 9. Documentation Consistency

README, CLI help, architecture, threat model, and evidence must agree on:

- implemented adapters;
- supported commands;
- network restrictions;
- scale actually tested;
- metrics actually reported;
- common-funder limitation;
- safety exclusions;
- draft status.

No checklist is marked complete from a stub, compilation alone, mocked success, or an
instruction fixture without a successful Surfpool state transition.

## 10. Draft Exit Gate

The PR can leave draft only after:

- all mandatory commands pass twice from clean Surfpool state;
- a fresh clone follows the quick start successfully;
- every P0 adapter has a Surfpool signature and state assertion;
- recovery proof is complete;
- evaluator correctness fixtures pass;
- a comparative report is committed;
- unsupported privacy claims are absent;
- sponsor eligibility questions are resolved;
- Marcelo approves final submission.

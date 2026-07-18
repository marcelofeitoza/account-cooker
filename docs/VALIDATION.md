# Validation And Evidence Plan

Status: implemented acceptance contract; reduced full-demo rehearsal passed, canonical
clean-clone evidence pending.

## 1. Quality Gates

The draft PR remains draft until these pass from a clean checkout:

    cargo fmt --all --check
    cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
    cargo test --locked --workspace --all-features
    cargo test --locked --workspace --doc
    bash -n scripts/*.sh scripts/lib/*.sh
    shellcheck -x -P SCRIPTDIR scripts/*.sh scripts/lib/*.sh
    cargo audit
    cargo deny check
    gitleaks dir --redact --no-banner --verbose .
    gitleaks git --redact --no-banner --verbose .
    ./scripts/surfpool-chain-acceptance.sh
    ./scripts/full-demo.sh

`scripts/full-demo.sh` is the executable superset. Its default mode requires a clean tree;
`--quick` exists only to rehearse the harness at reduced dimensions.

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

| Scenario | Proof boundary | Required assertion |
|---|---|---|
| network guard | unit plus live doctor | public/non-Surfpool RPC fails before signer load |
| clean bootstrap | full demo | generated fleet, funding, manifest, and balances agree |
| native transfer | Surfpool acceptance | exact source, destination, and fee deltas |
| SPL transfer | Surfpool acceptance | ATA creation, ownership, mint, and exact token delta |
| Jupiter swap | pinned Surfpool state | bounded deltas, allowlisted route, local signer rebinding |
| native stake lifecycle | Surfpool acceptance | create, delegate, deactivate, withdraw state transitions |
| insufficient funds | unit and Surfpool fault acceptance | no unauthorized signature or state promotion |
| stale blockhash | Surfpool fault acceptance | same-signature expiry/reconciliation, no blind rebuild |
| simulation failure | Surfpool fault acceptance | no send attempted |
| response lost after send | Surfpool soak | one landed signature recovered without resend |
| six process checkpoints | SQLite runtime integration | restart reaches one valid terminal result |
| historical rollback | runtime/store integration plus live re-audit | orphan correction never resubmits |
| Surfpool restart | Surfpool soak | resume same database/Surfnet with no funding reset |
| concurrent workers | runtime integration plus Surfpool soak | bounded workers and one wallet lease |
| budget exhaustion | policy/property tests | no transaction signature beyond limit |
| kill switch | CLI/runtime tests | no network, signer load, claim, or state change |
| compressed soak | Surfpool soak | zero duplicate or unresolved actions/signatures |

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
- at least 1,000 locally executed native transactions in the canonical faulted soak;
- separate native, SPL, Jupiter, stake, and fault transactions in chain acceptance;
- restart and fault injection during the run;
- zero duplicate logical intents;
- zero budget violations;
- zero unresolved Submitted actions after final reconciliation.

Fewer than 1,000 confirmed soak transactions is a failed canonical gate. Correctness is
never weakened to manufacture that number.

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

The SQLite runtime integration suite injects failures at:

1. after intent persistence;
2. after simulation;
3. after prepared transaction persistence;
4. after local signature creation;
5. after send response is lost;
6. after confirmation but before local promotion.

For each checkpoint it closes and reopens the same database, then:

- stop the cooker without cleanup;
- preserve the application database;
- restart the runtime;
- execute or reconcile according to the persisted state;
- assert one logical ActionId;
- assert zero or one landed signature as appropriate;
- record the event sequence.

This exhaustive checkpoint proof uses a deterministic in-process chain boundary so every
side-effect window is reachable. The real Surfpool soak separately injects one response
loss only after the submitted signature is observable, reconstructs the runtime, restarts
Surfpool against the same persistent database, reconciles without resend, and proves exact
aggregate principal plus fee deltas. The full demo also re-audits every confirmed CLI
action without submitting another transaction.

## 7. Evidence Layout

Canonical committed layout:

    evidence/
      README.md
      final/
        manifest.json
        cli-workflow.json
        transactions.json
        virtual-soak.json
        surfpool-soak.json
        surfpool-session.json
        report.md
        metrics.json
        metrics.csv
        metrics.md
        test-summary.txt
        config.redacted.toml
        commands.txt
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
- sanitized local transaction signature samples;
- metric artifact hashes;
- known limitations.

Raw logs, mutable databases, generated signers, full signatures, signed transactions, and
decompressed snapshots remain ignored. Only validated, redacted, reviewed artifacts are
committed.

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
- every required adapter has a Surfpool signature and state assertion;
- recovery proof is complete;
- evaluator correctness fixtures pass;
- a comparative report is committed;
- unsupported privacy claims are absent;
- sponsor eligibility questions are resolved;
- Marcelo approves final submission.

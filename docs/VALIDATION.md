# Validation And Evidence Plan

Status: executable acceptance contract implemented and complete. The canonical clean-tree
run, two fresh-clone repetitions, and committed `evidence/final` pack passed. Reduced or
focused runs cannot be cited as canonical results.

## 1. Quality Gates

These gates passed from clean checkouts before PR 2 was marked ready for review:

    cargo fmt --all -- --check
    ./scripts/check-lint-policy.sh
    cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
    cargo test --locked --workspace --all-features
    cargo test --locked --workspace --doc
    bash -n scripts/*.sh scripts/lib/*.sh
    shellcheck -x -P SCRIPTDIR scripts/*.sh scripts/lib/*.sh
    cargo audit
    cargo deny check
    ./scripts/gitleaks-working-tree.sh
    gitleaks git --redact --no-banner --verbose .
    ./scripts/surfpool-chain-acceptance.sh
    ./scripts/full-demo.sh

`scripts/full-demo.sh` is the executable superset. Its default mode requires a clean tree;
`--quick` exists only to rehearse the harness at reduced dimensions.

`scripts/check-lint-policy.sh` is the machine-checkable form of two declared postures. It
fails if any Cargo target root drops its explicit `#![forbid(unsafe_code)]`, if a crate
stops inheriting the workspace lint table, if the workspace lint itself stops forbidding
unsafe code, or if `.cargo/audit.toml` stops denying advisory warnings. `cargo audit` reads
that same `.cargo/audit.toml`, so an unmaintained, unsound, or yanked dependency now fails
the gate instead of printing a warning; each accepted exception is listed there with a
reason and mirrors `deny.toml`. Both checks run in CI.

The committed `evidence/final` pack predates this gate. Its `quality-gates.json` therefore
has no `lint_policy` key, while a fresh canonical run emits one.

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
- execute every canonical successful chain path through Surfpool; the separate ignored
  devnet native-transfer test uses the same adapter through the public-cluster gateway;
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
| pooled funding routing | policy-aware mocked gateway runtime plus CLI preview | one global multi-fleet schedule preserves payer, denomination, round order, signer route, exact per-disburser budgets, and transfer count |
| native transfer | Surfpool acceptance | exact source, destination, and fee deltas |
| SPL transfer | Surfpool acceptance | ATA creation, ownership, mint, and exact token delta |
| Jupiter swap | pinned Surfpool state | bounded deltas, allowlisted route, local signer rebinding |
| native stake lifecycle | Surfpool acceptance | create, delegate, deactivate, withdraw state transitions |
| insufficient funds | unit and Surfpool fault acceptance | no unauthorized signature or state promotion |
| stale blockhash | Surfpool fault acceptance | same-signature expiry/reconciliation, no blind rebuild |
| simulation failure | Surfpool fault acceptance | no send attempted |
| response lost after send | Surfpool soak | one landed signature recovered without resend |
| six process checkpoints | real child processes plus Surfpool | `SIGKILL`, reopen, exact submit/signature/state deltas, one terminal result |
| historical rollback | runtime/store integration plus live re-audit | orphan correction never resubmits |
| Surfpool restart | Surfpool soak | resume same database/Surfnet with no funding reset |
| Surfpool profiler isolation | start/session provenance | instruction profiling disabled for canonical soak execution |
| Surfpool datasource isolation | start/session provenance | offline snapshot mode; no lazy public-RPC account misses |
| concurrent workers | runtime integration plus Surfpool soak | bounded workers and one wallet lease |
| budget exhaustion | policy/property tests | no transaction signature beyond limit |
| kill switch | CLI/runtime tests | no network, signer load, claim, or state change |
| compressed soak | Surfpool soak | zero duplicate or unresolved actions/signatures |
| public cluster identity | unit plus devnet run | application configuration and CLI cannot select a public cluster; the source-declared devnet gateway proves a pinned genesis hash |
| bounded public devnet run | ignored devnet test | named engine invariants on public devnet, with the confirmation rate measured rather than asserted |

## 4. Scale Runs

### Virtual run

- 1,000 agents;
- 30 virtual days;
- exactly the five canonical seeds `11`, `23`, `37`, `51`, and `71`;
- same agent definitions for baseline and modeled planner;
- deterministic output hash for each seed;
- bounded task count and streaming trace output;
- record wall time, peak memory when available, events, and database size.

### Surfpool run

- representative fleet subset for wall-clock operation;
- at least 1,000 locally executed native transactions in the canonical faulted soak;
- separate native, SPL, Jupiter, fault, process-recovery, and stake acceptance groups;
- restart and fault injection during the run;
- zero duplicate logical intents;
- zero budget violations;
- zero unresolved Submitted actions after final reconciliation.

Fewer than 1,000 confirmed soak transactions is a failed canonical gate. Correctness is
never weakened to manufacture that number.

### Bounded public devnet run

Separate from the canonical gate and never part of it, because the canonical pack is defined
to contain no public-network transaction. `scripts/devnet-soak.sh` runs the same store,
runtime engine, policy, and native-transfer adapter against `https://api.devnet.solana.com`.

Its gate asserts only what the engine controls:

- the endpoint proves devnet's genesis hash before a signer is loaded;
- every planned action reaches a terminal state, and the states sum to the planned count;
- zero duplicate logical intents and zero duplicate signatures;
- zero budget violations;
- zero unresolved Submitted or Unknown actions after reconciliation;
- the injected response loss reconciles to a confirmation without resubmission;
- exact source debit and destination credit for every confirmed action;
- the payer balance equation closes over confirmed transfers plus observed fees;
- one in-process reconstruction: the first runtime is dropped, then a new runtime is built
  with a reopened store handle, reloaded signer, fresh policy, adapter, context, and clock,
  plus the original gateway object; the Account Cooker OS process and Tokio runtime remain;
- evidence action records appear in ascending planned sequence order;
- every unconfirmed action carries a classified cause, and the causes sum to the unconfirmed
  count, so a non-inclusion can never be reported without a reason.

The confirmation rate is measured and published, never asserted. Inclusion is the cluster's
decision, not the engine's, so gating on it would reward tuning the number instead of
reporting it. Full signatures are committed so a reviewer can check both the confirmations
and the non-inclusions against the cluster directly.

Read [the devnet run and topology delta](DEVNET.md) before comparing the two runs. It states
what the run directly establishes and which live-network properties remain unmeasured.

## 5. Evaluation Runs

Compare:

- naive uniform planner;
- independent timing/weighted-action baseline;
- persona/session planner;
- one mitigation removed at a time.

Use the five held-out seeds `11`, `23`, `37`, `51`, and `71`, with equal:

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
- payer-and-round and scheme-aware funding attacks separately;
- every funding scheme against the dedicated-funder baseline under one shared
  participation schedule;
- mean, min, max, and per-seed values;
- action counts and rejected actions;
- failures and blind spots.

There is no predetermined requirement that the privacy hypothesis succeed. A result can
fail the claim gate while the runtime still passes its engineering gates.

## 6. Recovery Proof

The Surfpool process-recovery integration test starts a separate child for each checkpoint:

1. after intent persistence;
2. after prepared transaction persistence;
3. after simulation;
4. after local signature creation;
5. after send response is lost;
6. after confirmation but before local promotion.

For every case, the parent waits for a flushed checkpoint marker, sends `SIGKILL`, and
independently inspects the persisted store and Surfpool state. A second child then reopens
the same database and executes or reconciles. The gate requires:

- exactly one logical ActionId, prepared record, simulation, submission record, and trace;
- five confirmed actions with one landed signature and exact principal-plus-fee deltas;
- one locally signed but never submitted action that expires only after its last valid
  block height is crossed;
- exact crash-side and recovery-side submit-attempt counts;
- reuse of the same persisted prepared bytes and signature where they existed before the
  crash;
- zero duplicate local signatures and zero unresolved actions.

The independent Surfpool soak separately injects one response loss only after the submitted
signature is observable, reconstructs the runtime, restarts Surfpool against the same
persistent database, reconciles without resend, and proves exact aggregate principal plus
fee deltas. The full demo also re-audits every confirmed CLI action without submitting
another transaction. Native stake acceptance runs after both recovery proofs because its
epoch travel must be last on the independent chain-acceptance node.

## 7. Evidence Layout

Canonical committed layout:

    evidence/
      README.md
      final/
        manifest.json
        run.json
        executions.json
        quality-gates.json
        cli-workflow.json
        transactions.json
        virtual-soak.json
        surfpool-soak.json
        surfpool-provenance.json
        report.md
        metrics.json
        metrics.csv
        metrics.md
        test-summary.txt
        config.executed.toml
        reproduce.txt
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
- six chain-acceptance groups, including all six real-process crash cases;
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
- funding-provenance limitation and the schemes it was measured under;
- direct versus pooled runtime funding, including external pool deposits and synthetic-only
  measurement scope;
- safety exclusions;
- review status.

No checklist is marked complete from a stub, compilation alone, mocked success, or an
instruction fixture without a successful Surfpool state transition.

## 10. Completed Review Readiness Gate

PR 2 was marked ready for review only after:

- all mandatory commands pass twice from clean Surfpool state;
- a fresh clone follows the quick start successfully;
- every required adapter has a Surfpool signature and state assertion;
- recovery proof is complete;
- evaluator correctness fixtures pass;
- a comparative report is committed;
- unsupported privacy claims are absent;
- the published-policy eligibility audit is current;
- Marcelo approves the human submission.

The dated audit in `ELIGIBILITY.md` records that `HUMAN_ONLY` blocks agent-API submission,
while the listing and platform terms contain no prohibition on disclosed AI assistance by a
human entrant. Engineering completion did not establish Marcelo's country or account
eligibility; those attestations, the final review, and submission remained human controls.

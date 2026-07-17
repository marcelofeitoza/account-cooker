# Account Cooker Master Plan

Status: ready for implementation after plan review.

This document is the public implementation contract for a one-session build. It
defines what will be built, what evidence is required, which claims are permitted,
and which features are cut before correctness, Surfpool verification, or safety.

## 1. Objective

Build a production-shaped Rust account activity engine that:

1. Schedules thousands of persistent agents without one task per wallet.
2. Produces stateful, policy-constrained Solana actions through reusable adapters.
3. Recovers deterministically after process, RPC, and transaction failures.
4. Evaluates behavioral and graph leakage against declared attacker features.
5. Proves all chain-facing behavior through reproducible Surfpool scenarios.

The result must be useful even when the evaluator finds that a behavior model is
distinguishable. The system reports failed privacy hypotheses rather than hiding them.

## 2. Operating Contract

The following decisions are fixed for the first implementation:

- Language: Rust end to end.
- License: MIT.
- Identity: independent personal contribution.
- Development chain: Surfpool only.
- Upstream writes: development never submits to devnet or mainnet.
- Source: clean-room implementation from the bounty specification and public APIs.
- Runtime default: dry-run, followed by an explicit Surfpool execution mode.
- Claim policy: no anonymity or indistinguishability claim without evaluator evidence.
- Safety policy: no wash trading, self-trading, governance voting, referral farming,
  fake engagement, NFT bidding, dust spam, bridge churn, or uncontrolled live execution.
- Secret policy: no private key, seed, raw RPC credential, or unredacted trace in Git.

Unit and property tests may run in process. Every RPC path, adapter, confirmation path,
recovery path, and executable soak must also pass through Surfpool.

## 3. Definition Of Success

The one-session build is complete only when all P0 gates are green:

- A clean checkout builds with the pinned Rust toolchain.
- Format, Clippy with warnings denied, unit tests, and property tests pass.
- The CLI can create a deterministic fleet, plan actions, run them on Surfpool,
  stop, restart, reconcile, and continue without double execution.
- Native transfer, SPL transfer, and Jupiter swap complete on Surfpool.
- One stateful protocol lifecycle completes on Surfpool. Marinade is first choice;
  native stake is the bounded fallback if the external Marinade interface blocks.
- A 1,000-agent, 30-virtual-day simulation produces a deterministic trace.
- A compressed wall-clock Surfpool soak executes a bounded subset of that trace.
- The evaluator compares naive and modeled behavior over at least five seeds.
- An evidence manifest records commands, versions, seeds, Surfpool configuration,
  transaction signatures, test results, metrics, and known limitations.
- The README states only claims supported by the committed evidence.

## 4. Scope

### P0: prize-complete one-session build

- Rust workspace, configuration schema, error taxonomy, and structured logging.
- Deterministic persona/session generator and priority-queue scheduler.
- SQLite WAL store with schema migrations, leases, idempotency, and reconciliation.
- Controller and bounded worker runtime.
- Surfpool-only RPC guard and network bootstrap scripts.
- Ephemeral local signer provider with strict file permissions.
- Native SOL transfer, SPL transfer, Jupiter swap, and one stateful lifecycle.
- Policy engine with budgets, allowlists, cooldowns, slippage, and loss limits.
- Trace recorder and adversarial evaluator.
- Unit, property, integration, recovery, and soak tests.
- Architecture, threat model, Surfpool runbook, validation report, and full demo.

### P1: same-session stretch, only after every P0 gate

- A second stateful protocol adapter, preferably Orca LP or Marinade unstake.
- Prometheus endpoint and a compact terminal status view.
- Surfpool account-state scenarios for price shock, low liquidity, and blockhash expiry.
- Sanitized observed-traffic calibration fixture built through Surfpool.
- Criterion benchmarks for scheduler throughput and evaluator feature extraction.

### Explicit non-goals

- Mainnet or devnet bot execution.
- Hiding or laundering a common funding source.
- A ZK pool, mixer, relay, or external privacy-protocol integration.
- Cross-host consensus or a distributed database.
- DAO voting, airdrop farming, referral farming, NFT manipulation, or wash volume.
- Bridges, arbitrary plugin loading, or unreviewed protocol calls.
- Claims that random activity makes wallets anonymous.
- Reimplementation of private or third-party code.

## 5. Deliverables

| Deliverable | Purpose | Acceptance |
|---|---|---|
| cooker CLI | init, plan, run, status, recover, evaluate | deterministic JSON and human output |
| runtime | controller, scheduler, workers, reconciliation | restart test proves no duplicate action |
| store | migrations and durable lifecycle | WAL, idempotency, leases, receipt history |
| adapters | real transaction planning and observation | Surfpool signatures and state assertions |
| evaluator | attacker features and comparative metrics | five seeds, baseline, JSON/CSV/Markdown |
| Surfpool scripts | clean network and full demo | one command from clean checkout |
| evidence pack | reproducibility and claim basis | manifest contains versions and hashes |
| documentation | architecture, threat model, limitations | matches implemented behavior |

## 6. Planned Crate Graph

The initial workspace has six crates:

1. **cooker-core**
   - AgentId, ActionId, Persona, SessionState, PlannedAction, ActionOutcome.
   - Deterministic RNG derivation.
   - Behavior model, scheduler, policy types, and adapter contracts.
   - No RPC, database, CLI, or filesystem dependency.

2. **cooker-store**
   - Store trait and SQLite implementation.
   - Embedded migrations and WAL initialization.
   - Agent, action, attempt, receipt, lease, model, and run records.
   - Atomic state transitions and reconciliation queries.

3. **cooker-solana**
   - Surfpool RPC client and loopback-network guard.
   - SignerProvider and ephemeral file signer.
   - Blockhash refresh, fee policy, simulation, submit, confirm, and receipt parsing.
   - Native, SPL, Jupiter, and stateful adapter modules.

4. **cooker-runtime**
   - Controller, priority queue, bounded worker pool, cancellation, and recovery.
   - Plan -> simulate -> submit -> confirm -> observe lifecycle.
   - Structured events, counters, and trace recording.

5. **cooker-eval**
   - Stable trace schema.
   - Attacker feature extraction, baselines, clustering/classification metrics.
   - Seed aggregation, confidence intervals, ablations, and report generation.

6. **cooker-cli**
   - Commands: init, plan, run, status, recover, evaluate, doctor.
   - TOML configuration with environment overrides for non-secret values.
   - JSON output for automation and concise human output for operators.

Crates may be collapsed if compile boundaries slow delivery without clarifying ownership.
The domain, store, chain, runtime, evaluation, and CLI ownership boundaries remain.

## 7. Core Interfaces

### Action adapter

Each adapter owns a complete stateful lifecycle:

1. discover current accounts and balances;
2. quote or derive a bounded action;
3. produce a typed plan with expected state changes;
4. simulate against Surfpool;
5. execute with an idempotency key;
6. confirm at the configured commitment;
7. observe actual deltas and protocol receipts;
8. reconcile after an unknown or interrupted outcome.

An adapter never marks success from sendTransaction alone. Confirmation and observed
postconditions are part of the contract.

### Store

The store exposes transactionally safe operations:

- create or update an agent;
- enqueue a planned action once;
- claim the next due action with a lease;
- record simulation;
- record submitted signature before waiting for confirmation;
- promote to confirmed only after observation;
- classify deterministic, transient, policy, and unknown failures;
- release expired leases;
- find submitted actions requiring reconciliation;
- append immutable events used by the evaluator.

### Policy

Policy returns Allow, Delay, Rewrite, or Reject. It checks:

- environment is Surfpool;
- signer and destination are allowlisted;
- action and protocol are enabled;
- daily and lifetime budget remain;
- reserve and rent floors remain;
- fee, slippage, and price-impact bounds hold;
- cooldown and sequence rules hold;
- no known self-trade or prohibited action exists;
- consolidation does not violate configured graph constraints.

## 8. Action State Machine

Actions move through:

Planned -> Simulated -> Submitted -> Confirmed

Terminal alternatives:

- Rejected: policy or deterministic validation failure.
- Failed: confirmed chain failure.
- Expired: no longer valid before submission.
- Unknown: submission may have landed; reconciliation is mandatory.
- Orphaned: previously observed result is absent after network rollback.
- Cancelled: operator shutdown before submission.

Rules:

- ActionId is deterministic from run, agent, logical sequence, and model version.
- A Submitted action is never submitted again until signature reconciliation finishes.
- Every state change and attempt is append-only in the event table.
- Store state is advisory; Surfpool chain state and signature history are authoritative.
- Shutdown stops new claims, lets bounded in-flight work settle, then persists a checkpoint.

## 9. Behavior Model

### Persona

Each agent has immutable identity parameters:

- deterministic seed and wallet alias;
- timezone and weekly activity profile;
- balance and risk bands;
- protocol affinities;
- amount roundness and size mixture;
- session duration and hold-time distributions;
- cooldown, skip-day, and inactivity probabilities.

### Semi-Markov session states

The P0 model uses:

- Dormant
- Active
- Transacting
- Holding
- CoolingDown

Transitions depend on persona, local time, prior actions, balances, and policy. Durations
are sampled separately from transitions so the engine does not become a memoryless loop.

### Scheduling

- One priority queue orders all due actions.
- A bounded worker pool executes them.
- No task is held open per wallet.
- The master seed comes from OS entropy by default.
- Each agent derives an independent ChaCha stream from master seed and AgentId.
- Tests and evidence runs pin the master seed.
- Model version and all calibration inputs are stored with each plan.

### Amounts and sequences

- Amounts use mixtures with explicit round-number mass and bounded heavy tails.
- Rare observed values are never copied exactly.
- Sequence probabilities are conditional, not independent weighted draws.
- Immediate round trips and deterministic high-balance-to-low-balance rebalancing are
  prohibited by policy.
- Hold time and consolidation are first-class actions because longitudinal linkage is
  more important than a single transaction's appearance.

## 10. Adapter Order

1. **Native transfer**
   - Establish transaction, confirmation, receipt, and recovery plumbing.
   - Assert exact source, destination, and fee deltas on Surfpool.

2. **SPL transfer**
   - Clone or create mint and token accounts through Surfpool.
   - Assert mint, owner, ATA, decimals, and token balance changes.

3. **Jupiter swap**
   - Use the public quote/instruction interface.
   - Rebuild with a Surfpool blockhash and local signer.
   - Simulate before submit.
   - Assert input/output balance ranges, route program allowlist, and slippage.

4. **Marinade lifecycle**
   - Preferred P0 stateful integration.
   - Deposit/stake and observe the resulting position or liquid-staking balance.
   - If current public interface or Surfpool program state blocks it at the adapter
     checkpoint, implement native stake create/delegate/deactivate as the bounded
     stateful fallback and record the blocker.

5. **Orca LP**
   - P1 only. Position creation and removal must be modeled as a lifecycle, not two
     unrelated transactions.

## 11. Observer And Calibration

The observer consumes only Surfpool's RPC endpoint. It may use Surfpool's lazy mainnet
datasource, but application code never connects to a remote Solana RPC directly.

P0 inputs:

- deterministic synthetic ground truth for correctness;
- traces generated by the naive baseline;
- traces generated by the modeled scheduler;
- chain receipts generated inside Surfpool.

P1 may add a sanitized held-out feature fixture. Raw transactions are converted to
non-identifying feature rows before committing. The fixture records the source window,
extractor version, filters, and limitations.

Surfpool does not guarantee complete historical transaction bodies for every lazy-cloned
account. Historical calibration is therefore optional. Fresh transactions produced inside
the Surfnet are the authoritative chain-facing test corpus.

## 12. Adversarial Evaluator

### Declared attacker features

- common funding ancestor and funding depth;
- common fee payer or signer reuse;
- amount uniqueness, roundness, and relative balance fraction;
- inter-arrival time, active hours, periodicity, and burst structure;
- protocol and instruction sequence;
- shared destinations and token accounts;
- consolidation delay and fan-in shape;
- balance-rank transitions;
- venue, route, slippage, and fee patterns;
- session-length and inactivity patterns.

### Baselines

- naive uniform timing and amount model;
- independent log-normal timing with weighted actions;
- the proposed persona/session model;
- ablations removing one mitigation at a time.

### Metrics

- per-feature distribution distances;
- attacker precision, recall, F1, and ROC AUC where labels exist;
- adjusted Rand index or normalized mutual information for wallet clustering;
- top-k ownership-link recovery;
- baseline-relative improvement over multiple seeds;
- confidence interval or seed range, never a single favorable seed.

### Claim gate

A favorable privacy statement may appear in the README only if:

1. the metric and baseline were declared before the final run;
2. all compared runs use the same budgets and action counts;
3. at least five deterministic seeds were evaluated;
4. the direction of improvement is consistent across seeds;
5. the result and known blind spots are committed in an evidence pack.

Otherwise the report says that no supported improvement was established.

## 13. Surfpool Development Contract

The canonical RPC endpoint is http://127.0.0.1:8899. P0 code rejects non-loopback
Solana RPC URLs. The exact workflow is documented in docs/SURFPOOL.md.

Required scenarios:

- fresh fleet bootstrap and funding;
- native and SPL success;
- Jupiter success and rejected slippage;
- stateful lifecycle success;
- insufficient balance;
- expired or stale blockhash;
- transient RPC failure before submit;
- unknown response after submit;
- process crash after submit and before confirmation persistence;
- restart and reconciliation;
- policy rejection;
- accelerated multi-agent soak.

Every integration test starts or targets a named, isolated Surfnet and records:

- Surfpool version;
- datasource mode;
- Surfnet ID;
- snapshot or scenario hashes;
- slot and blockhash context;
- generated account aliases;
- submitted signatures and final errors;
- observed pre/post state.

## 14. Security And Abuse Controls

- Surfpool-only endpoint validation is enforced in code, not just documentation.
- Generated local key files are mode 0600 and ignored by Git.
- Logs contain aliases and public keys only where necessary; no secret material.
- Config redaction is tested.
- Protocol and program IDs are allowlisted.
- Destinations are limited to generated local fleet accounts and protocol-owned accounts.
- Budgets cover principal, fees, slippage, price impact, and daily action count.
- Dry-run and simulation are the default.
- A filesystem kill switch and Ctrl-C stop new work.
- The runtime records generated traffic separately from any observed reference fixture.
- No output calls generated actions organic user activity.

## 15. Validation Matrix

| Layer | Tool | Required result |
|---|---|---|
| formatting | cargo fmt | clean |
| lint | cargo clippy, warnings denied | clean |
| unit | cargo test | all pass |
| properties | proptest | conservation, ordering, idempotency, bounds |
| store | SQLite temp DB | migrations and transitions pass |
| scheduler | deterministic virtual clock | stable trace for fixed seed |
| RPC | mocked transport plus Surfpool | retries classified correctly |
| adapters | Surfpool | signatures and postconditions proven |
| recovery | Surfpool plus forced kill | no duplicate action |
| scale | virtual 1,000 agents x 30 days | bounded memory and deterministic count |
| soak | compressed Surfpool execution | no unreconciled submitted action |
| evaluator | known ground truth | metric fixtures and ablations pass |
| supply chain | cargo deny if time permits | no rejected license/source |

The complete evidence requirements are in docs/VALIDATION.md.

## 16. One-Session Execution Graph

This is a dependency graph, not a promise to wait for clock boundaries. Independent work
streams run in parallel after the domain contract is fixed.

### Block A: foundation, 0:00 to 1:30

- Create Rust workspace, toolchain, lint configuration, and CI.
- Define domain types, errors, config, trace schema, and adapter/store contracts.
- Add deterministic clock and RNG test harness.
- Start the canonical Surfpool network and doctor check.

Gate A: workspace is green and Surfpool health check passes.

### Block B: state and scheduling, 1:30 to 3:30

- Implement SQLite migrations and Store.
- Implement priority scheduler, persona states, policy skeleton, and deterministic planner.
- Add 1,000-agent virtual simulation and state-machine properties.

Gate B: same seed produces byte-identical trace; store restart preserves next actions.

### Block C: chain execution, 3:30 to 6:30

- Implement Surfpool RPC guard, signer, blockhash, simulate, submit, confirm, observe.
- Complete native and SPL adapters.
- Add unknown-outcome reconciliation and crash injection.
- Complete Jupiter adapter.

Gate C: all three action types pass from a clean Surfnet with state assertions.

### Block D: stateful adapter and evaluator, 6:30 to 9:30

- Spike and implement Marinade lifecycle, or activate native-stake fallback at checkpoint.
- Implement feature extraction, baselines, metrics, seed aggregation, and ablations.
- Connect chain receipts to stable trace rows.

Gate D: one stateful lifecycle and evaluator known-ground-truth tests pass.

### Block E: hardening and proof, 9:30 to 12:00

- Run restart, fault, virtual-scale, and compressed Surfpool soak scenarios.
- Run fmt, Clippy, full tests, optional deny/bench.
- Generate evidence pack and full-demo output.
- Update implementation status, limitations, architecture, and draft PR.

Gate E: no P0 failure, no unsupported claim, no secret in Git, clean worktree.

## 17. Scope-Cut Rules

Cuts happen in this order:

1. Orca stretch adapter.
2. Prometheus UI and benchmarks.
3. Historical observed-traffic calibration.
4. Marinade, replaced by tested native stake lifecycle.
5. Advanced learned classifier, retaining transparent heuristic metrics.

Never cut:

- Surfpool execution;
- deterministic tests;
- state recovery and idempotency;
- endpoint and budget guardrails;
- evaluator known-ground-truth correctness;
- evidence manifest;
- honest limitation documentation;
- format and Clippy gates.

No blocked adapter is represented as working. A golden instruction fixture is not a
substitute for a successful Surfpool state transition.

## 18. Commit And PR Sequence

The draft PR remains reviewable through small, ordered commits:

1. docs: define measured account-cooker implementation plan
2. chore: scaffold Rust workspace and quality gates
3. feat: add deterministic personas and fleet scheduler
4. feat: add durable action state and restart recovery
5. feat: add Surfpool runtime and transfer adapters
6. feat: add Jupiter and stateful protocol lifecycles
7. feat: add adversarial evaluator and comparative reports
8. test: add Surfpool recovery and scale evidence
9. docs: publish reproducible demo and supported claims

The PR stays draft until all P0 gates pass. The initial draft contains planning material
only and must not be interpreted as an implemented feature.

## 19. Sponsor Clarifications

These questions do not block the safe local build, but they affect final submission:

- Does HUMAN_ONLY govern only the submitting profile, or AI-assisted implementation too?
- Must the PR be merged, or only publicly reviewable, before the deadline?
- Is Surfpool proof sufficient, or is a public devnet/mainnet transaction expected?
- What exact attacker and metric should satisfy statistically indistinguishable?
- Are unsafe examples such as governance voting and artificial protocol activity optional?
- Does Rust end to end allow shell runbooks and generated JSON evidence?
- Can one participant win more than one repository track?

## 20. Final Definition Of Done

Done means a new reviewer can:

1. clone the repository;
2. run one documented bootstrap command;
3. start a clean Surfpool network;
4. execute the full demo without remote Solana writes;
5. inspect successful signatures and state deltas;
6. kill and restart the daemon during a submitted action;
7. verify that recovery does not duplicate it;
8. reproduce the 1,000-agent trace and evaluator report;
9. understand exactly which privacy properties were and were not measured;
10. verify that the Git history contains no external private code, secrets, or
    unsupported claims.

Anything less remains a draft.

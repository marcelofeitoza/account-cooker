# Account Cooker Master Plan

Status: Blocks A-F, the canonical release proof, the published-policy eligibility audit,
and the PR handoff are complete. PR 2 is open and ready for review.

This document is the public implementation and release contract for the bounty build. It
defines what is implemented, what evidence is still required, which claims are permitted,
and which features remain outside the completion boundary.

## Completion State

| Block | Scope | State |
|---|---|---|
| A | pinned workspace, CI, configuration, CLI, Surfpool harness | complete |
| B | personas, scheduler, SQLite durability, policy, concurrency, properties | complete |
| C | native SOL, SPL, Jupiter, and native stake through Surfpool | complete |
| D | crash, unknown outcome, restart, rollback audit, budget, kill switch | complete |
| E | evaluator, security/provenance, public docs, Obsidian record | complete |
| F | canonical soaks, sanitized evidence, two clean-clone proofs, PR handoff | complete |

Blocks A-E describe implemented source and focused verification. Block F adds clean-tree
canonical evidence, two independent clean-clone reproductions, and the public PR handoff.

## 1. Objective

Build a production-shaped Rust account activity engine that:

1. Schedules the 1,000-agent canonical fleet without one task per wallet.
2. Produces stateful, policy-constrained Solana actions through reusable adapters.
3. Recovers deterministically after process, RPC, and transaction failures.
4. Evaluates behavioral and graph leakage against declared attacker features.
5. Proves all canonical chain-facing behavior through reproducible Surfpool scenarios.

The result must be useful even when the evaluator finds that a behavior model is
distinguishable. The system reports failed privacy hypotheses rather than hiding them.

## 2. Operating Contract

The following decisions are fixed for the complete deliverable:

- Language: Rust end to end.
- License: MIT.
- Identity: independent personal contribution.
- Development chain: Surfpool for all development and all canonical evidence.
- Upstream writes: never mainnet. One opt-in bounded devnet run exists to answer whether the
  engine holds on public-network topology; it is declared in source, excluded from the
  canonical run and from CI, and recorded separately in `evidence/devnet`.
- Source: standalone clean-room implementation from the bounty specification and public
  APIs, with no code, fixture, service, key, data, or runtime dependency on any private
  project.
- Runtime default: dry-run, followed by an explicit Surfpool execution mode.
- Claim policy: no anonymity or indistinguishability claim without evaluator evidence.
- Safety policy: no wash trading, self-trading, governance voting, referral farming,
  fake engagement, NFT bidding, dust spam, bridge churn, or uncontrolled live execution.
- Secret policy: no private key, seed, raw RPC credential, or unredacted local trace in Git.
  The separate devnet record commits public transaction signatures for verification.

Unit and property tests may run in process. Every canonical adapter, confirmation, recovery,
and executable soak path must also pass through Surfpool. The sole public-network exception
is the source-declared, ignored devnet native-transfer run described in `docs/DEVNET.md`.

## 3. Definition Of Success

The bounty build is complete only when every mandatory gate is green:

- A clean checkout builds with the pinned Rust toolchain.
- Format, Clippy with warnings denied, unit tests, and property tests pass.
- The CLI can create a deterministic fleet, plan actions, run them on Surfpool,
  stop, restart, reconcile, and continue without double execution.
- Native transfer, SPL transfer, and Jupiter swap complete on Surfpool.
- A native stake create/delegate/deactivate/withdraw lifecycle completes on Surfpool.
- Six real child processes are stopped with `SIGKILL` at distinct durable lifecycle
  checkpoints and recover against the same SQLite database and Surfpool without a
  duplicate submit, signature, logical action, or event trace.
- A 1,000-agent, 30-virtual-day simulation produces deterministic traces for the five
  canonical seeds.
- A compressed wall-clock Surfpool soak executes at least 1,000 bounded native transfers
  with injected response loss and restart recovery.
- The evaluator compares naive and modeled behavior over the five canonical seeds.
- An evidence manifest records commands, versions, seeds, Surfpool configuration,
  transaction signatures, test results, metrics, and known limitations.
- The README states only claims supported by the committed evidence.

## 4. Scope

### Required complete bounty deliverable

- Rust workspace, configuration schema, error taxonomy, and structured logging.
- Deterministic persona/session generator and priority-queue scheduler.
- SQLite WAL store with schema migrations, leases, idempotency, and reconciliation.
- Controller and bounded worker runtime.
- Loopback-only application configuration guard, source-declared devnet test boundary, and
  Surfpool network bootstrap scripts.
- Ephemeral local signer provider with strict file permissions.
- Native SOL transfer, SPL transfer, Jupiter swap, and one stateful lifecycle.
- Policy engine with budgets, allowlists, cooldowns, slippage, and loss limits.
- Trace recorder and adversarial evaluator.
- Unit, property, integration, recovery, and soak tests.
- Architecture, threat model, Surfpool runbook, validation report, and full demo.

### Explicitly deferred extensions

These are possible follow-on projects, not completion gates and not implied capabilities:

- A second stateful protocol adapter, preferably Orca LP or Marinade unstake.
- Prometheus endpoint and a compact terminal status view.
- Surfpool account-state scenarios for price shock, low liquidity, and blockhash expiry.
- Sanitized observed-traffic calibration fixture built through Surfpool.
- Criterion benchmarks for scheduler throughput and evaluator feature extraction.

### Explicit non-goals

- Mainnet execution of any kind.
- Unattended or continuous devnet bot operation. The one devnet run is a bounded,
  hand-started measurement, not a deployed workload.
- Pool custody or deposit-side provenance mitigation. The runtime can execute the measured
  disbursement schedule across multiple public fleet manifests and already-funded local
  disbursers, but it does not source pool deposits or implement a custodial or on-chain
  mixing service.
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

## 6. Crate Graph

The workspace has six crates:

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
   - Commands: keygen, fleet-init, fund, init, validate, plan, simulate, evaluate, soak,
     doctor, run, status, and recover.
   - TOML configuration with environment overrides for non-secret values.
   - JSON output for automation and concise human output for operators.

These domain, store, chain, runtime, evaluation, and CLI ownership boundaries are the
implemented workspace graph.

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

The behavior model uses:

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

## 10. Implemented Adapter Boundaries

1. **Native transfer**
   - Establish transaction, confirmation, receipt, and recovery plumbing.
   - Assert exact source, destination, and fee deltas on Surfpool.

2. **SPL transfer**
   - Clone or create mint and token accounts through Surfpool.
   - Assert mint, owner, ATA, decimals, and token balance changes.

3. **Jupiter swap**
   - Use a reviewed quote/instruction fixture and a hash-pinned 21-account Surfpool
     snapshot for deterministic canonical acceptance.
   - Rebind only the fixture signer and its WSOL/USDC ATAs, then rebuild with a current
     Surfpool blockhash and fresh local signer.
   - Simulate before submit.
   - Assert input/output balance ranges, route program allowlist, and slippage.
   - Retain live public quote/instruction retrieval only as an explicitly selected
     planning mode, not as canonical evidence.

4. **Native stake lifecycle**
   - Create and initialize a stake account, delegate it to a local vote account, advance
     the Surfpool clock, deactivate, advance again, and withdraw.
   - Observe stake state and exact lamport postconditions at every transition.
   - Keep this stateful path deterministic and entirely inside the pinned Surfpool state.

5. **Orca LP**
   - Explicitly deferred. Any future implementation must model position creation and
     removal as one lifecycle, not two unrelated transactions.

## 11. Observer And Calibration

The observer consumes only Surfpool's loopback RPC endpoint. Canonical runs load the pinned
reviewed snapshot in Surfpool offline mode, so neither the observer nor Surfpool performs a
remote chain-RPC lookup.

Required inputs:

- deterministic synthetic ground truth for correctness;
- traces generated by the naive baseline;
- traces generated by the modeled scheduler;
- chain receipts generated inside Surfpool.

An extension may add a sanitized held-out feature fixture. Raw transactions are converted to
non-identifying feature rows before committing. The fixture records the source window,
extractor version, filters, and limitations.

Surfpool does not guarantee complete historical transaction bodies for every lazy-cloned
account. Historical calibration is therefore optional. Fresh transactions produced inside
the Surfnet are the authoritative chain-facing test corpus.

## 12. Adversarial Evaluator

### Declared attacker features

- common funding ancestor and funding depth;
- shared payer and batching round read together;
- scheme-aware co-funding weighted by how few accounts shared a batch;
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

The interactive default RPC endpoint is http://127.0.0.1:8899; isolated acceptance and
full-demo runs default to port 18899. Code rejects non-loopback Solana RPC URLs supplied as
configuration and then requires the exact pinned Surfpool identity. The one public-cluster
constructor is unreachable from configuration and proves a pinned genesis hash instead. The
workflow is documented in docs/SURFPOOL.md, and the public-network run in docs/DEVNET.md.

The chain-acceptance script executes six ordered groups on fresh isolated state:

1. native SOL with exact principal and fee attribution;
2. reviewed-state Jupiter exact-input with signer/ATA rebinding and bounded deltas;
3. classic SPL with ATA creation, exact token/native deltas, and historical re-audit;
4. insufficient-funds rejection, simulation failure, stale-blockhash expiry, and
   same-signature unknown-outcome reconciliation;
5. real-process crash/restart recovery at six persistence/side-effect checkpoints;
6. native stake create/delegate/deactivate/epoch-advance/withdraw with exact phase deltas
   and confirmation re-audits.

Stake is deliberately last because its epoch travel is irreversible inside that Surfpool
process. The separate compressed soak proves bounded concurrent execution, one lost send
response, runtime reconstruction, persistent Surfpool restart, and reconciliation.

Every Surfpool chain-acceptance group starts or targets a named, isolated Surfnet and records:

- Surfpool version;
- offline datasource mode;
- Surfnet ID;
- snapshot or scenario hashes;
- slot and blockhash context;
- generated account aliases;
- submitted signatures and final errors;
- observed pre/post state.

## 14. Security And Abuse Controls

- Application configuration enforces loopback Surfpool endpoints in code. The separate
  ignored devnet test constructs a named public cluster in source and verifies its genesis
  hash before loading a signer.
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
| supply chain | cargo audit and cargo deny | no unacknowledged advisory or rejected license/source |

The complete evidence requirements are in docs/VALIDATION.md.

## 16. Completion Blocks

### Blocks A-D: implemented

- The pinned workspace, six-crate ownership boundaries, configuration, CI, and fail-closed
  Surfpool harness are present.
- Deterministic personas, the bounded scheduler, policy engine, SQLite WAL lifecycle,
  leases, journals, audits, properties, and concurrency controls are present.
- Native SOL, classic SPL, reviewed-state Jupiter, and native stake adapters execute
  through the common Surfpool gateway with protocol-specific postconditions.
- Recovery covers stale blockhash, simulation failure, insufficient funds, unknown send
  response, historical re-audit, persistent Surfpool restart, and six real `SIGKILL`
  subprocess checkpoints.

### Block E: implementation hardening and records complete

- The evaluator, five-seed experiment, supply-chain gates, secret scans, evidence builder,
  and public documentation are present.
- The separately indexed Obsidian project and session records are current. They remain
  operator memory only and are not a source or runtime dependency of this repository.
- The expanded reduced full demo, six process-crash cases, and persistent Surfpool restart
  pass. Canonical-scale Block F artifacts are the sole basis for final scale claims.

### Block F: complete

- Completed exactly five seeds over 1,000 agents for 30 virtual days from a clean commit.
- Completed 1,000 locally signed Surfpool soak transactions with response loss, runtime
  reconstruction, persistent Surfpool restart, and reconciliation without resend.
- Generated and reviewed the sanitized checksum-bearing `evidence/final` pack.
- Reproduced the complete canonical command twice from fresh clones and fresh offline
  Surfpool state; deterministic metrics and all five virtual trace hashes match.
- Pushed the implementation and evidence commits and updated PR 2 with measured
  results, limitations, provenance, and reviewer entry points.
- Audited the live listing, platform agent rules, and terms. `HUMAN_ONLY` requires Marcelo's
  human submission path; no published rule prohibits disclosed AI assistance.

Committed canonical evidence references its exact clean source commit and both clean-clone
runs pass. Published-policy eligibility is documented in `docs/ELIGIBILITY.md`. Marcelo's
approval is a deliberately retained human release control, not an unfinished engineering
or evidence gate.

## 17. Fixed Scope Boundary

The complete bounty deliverable retains Surfpool execution, deterministic testing,
state recovery and idempotency, endpoint and budget guardrails, evaluator correctness,
sanitized evidence, and honest limitations. None can be removed to make a failing
canonical run appear successful.

Orca LP, Prometheus/UI work, benchmarks, historical observed-traffic calibration,
advanced learned classifiers, and Marinade remain unimplemented extensions. Native Solana
stake is the tested stateful fallback and is not represented as Marinade. A reviewed
instruction fixture is accepted only with a successful Surfpool simulation, confirmation,
and state transition.

## 18. PR And Evidence State

PR 2 contains the complete implementation, measured result, limitations, canonical evidence
links, and clean-clone verification. It is open and ready for review. Only the
checksum-bearing canonical pack tied to its exact clean commit supports scale, transaction,
or metric claims.

## 19. Eligibility Audit And Sponsor Questions

The dated audit in `docs/ELIGIBILITY.md` establishes the published submission contract:
Brazilian builders, Rust end to end, open-source MIT work, and human submission. Superteam's
agent documentation defines agent eligibility as `AGENT_ALLOWED` or `AGENT_ONLY`, so this
listing's `HUMAN_ONLY` value blocks agent-API submission. Neither the listing nor platform
terms prohibit AI-assisted implementation by a human entrant. The PR discloses assistance,
and no agent submission was attempted.

These questions were open at engineering handoff and did not relax any safety or evidence
gate:

- Must the PR be merged, or only publicly reviewable, before the deadline?
- What exact attacker and metric should satisfy statistically indistinguishable?
- Are unsafe examples such as governance voting and artificial protocol activity optional?
- Does Rust end to end allow shell runbooks and generated JSON evidence?

## 20. Final Definition Of Done

Done means a new reviewer can:

1. clone the repository;
2. run one documented bootstrap command;
3. start a clean Surfpool network;
4. execute the full demo without remote Solana writes;
5. inspect successful signatures and state deltas;
6. reproduce the injected response-loss and persistent Surfpool-restart recovery proof;
7. verify that recovery does not duplicate a logical action or signature;
8. reproduce the 1,000-agent trace, 1,000-transaction soak, and evaluator report;
9. verify the same result from two fresh clones and clean Surfpool states;
10. understand exactly which privacy properties were and were not measured;
11. verify that the Git history contains no external private code, secrets, or
    unsupported claims.

All eleven conditions are satisfied by the canonical pack and its two clean-clone
reproductions. PR 2 is open and ready for review.

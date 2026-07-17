# Architecture

Status: design contract; implementation has not started.

## 1. System Shape

Account Cooker separates pure behavior planning from durable orchestration, Solana
execution, and privacy evaluation:

    configuration
         |
         v
    persona factory -> agent snapshots -> durable store
         |                                  |
         v                                  v
    pure planner -> policy -> priority scheduler -> bounded workers
                                                  |
                                                  v
                                         action adapter
                                                  |
                                                  v
                                        Surfpool gateway
                                                  |
                                                  v
                                         receipt observer
                                                  |
                             +--------------------+--------------------+
                             |                                         |
                             v                                         v
                       durable event log                         evaluator trace

The same planner runs against a VirtualClock and a simulated adapter for fast,
deterministic fleet generation. Chain-facing behavior always uses Surfpool.

## 2. Architectural Properties

- One priority queue for the fleet, not one long-lived task per wallet.
- Pure planning before side effects.
- A durable intent exists before any transaction is built.
- Signed bytes and signature are persisted before uncertain submission handling.
- Chain observation, not an RPC send response, promotes an action to Confirmed.
- Unknown outcomes reconcile before any logical retry.
- Adapter-specific protocol logic cannot bypass global policy.
- Evaluator labels are isolated from feature extraction.
- Every report identifies model, extractor, configuration, and seed versions.

## 3. Crate Ownership

### cooker-core

Owns stable domain contracts:

- AgentId, RunId, ModelId, ActionId, AttemptId.
- Persona and SessionState.
- AgentSnapshot and BalanceSnapshot.
- ActionIntent, PlannedAction, PreparedAction, ActionReceipt.
- ActionKind, ProtocolId, AssetId, Budget, PolicyDecision.
- BehaviorModel, ActionAdapter, PolicyEngine, Clock contracts.
- Deterministic seed derivation and virtual scheduling.

It depends only on general-purpose crates. It does not know SQLite, JSON-RPC,
Solana client types, filesystem paths, or CLI arguments.

### cooker-store

Owns persistence:

- Store trait and SQLite implementation.
- Embedded versioned migrations.
- WAL mode and busy timeout.
- Atomic lease claims and state transitions.
- Immutable action event journal.
- Reconciliation queries and run checkpoints.

The first store is single-host. The Store trait preserves a future path to Postgres
without pretending SQLite provides cross-host coordination.

### cooker-solana

Owns all Solana-specific behavior:

- SurfpoolRpcUrl validated type.
- Surfpool identity and health probes.
- Solana RPC and WebSocket transport.
- SignerProvider and local ephemeral signer.
- Blockhash, transaction, fee, simulation, submission, confirmation, observation.
- Native, SPL, Jupiter, and stateful adapter modules.
- Typed protocol allowlists and postcondition checks.

No other crate accepts an arbitrary Solana RPC URL.

### cooker-runtime

Owns orchestration:

- Controller startup and shutdown.
- Priority queue and due-action dispatch.
- Bounded worker pool and per-wallet exclusivity.
- Retry classification and backoff.
- Unknown-outcome reconciliation.
- Graceful shutdown and crash-injection checkpoints.
- Structured events and runtime counters.

### cooker-eval

Owns measurement:

- Stable, versioned observation and label formats.
- Feature extraction.
- Transparent attacker heuristics.
- Pairwise and cluster metrics.
- Baselines, ablations, seed aggregation, and report rendering.

The evaluator accepts public observations and a separate ground-truth label stream.
Feature extractors cannot read controller ownership labels.

### cooker-cli

Owns operator interaction:

- Configuration loading and validation.
- init, doctor, plan, simulate, run, status, recover, evaluate.
- Human-readable and machine-readable output.
- Exit codes and policy acknowledgement.

## 4. Stable Domain Model

### Agent

An agent record contains:

- stable AgentId;
- public wallet identity and signer reference;
- Persona and model version;
- deterministic sequence number;
- lifecycle status;
- budget counters;
- last observed balances;
- next due time;
- last confirmed action;
- lease owner and expiry when claimed.

Secrets are never stored in the agent table.

### Persona

A Persona contains immutable parameters:

- timezone;
- active-day and active-window distributions;
- daily participation probability;
- session duration and action-count distributions;
- session-state transition matrix;
- amount mixture and roundness mass;
- action affinity conditioned on session state;
- holding-time and cooldown distributions;
- balance reserve and risk limits.

Configuration validation enforces normalized probabilities, bounded supports, valid time
windows, and nonzero safety reserves.

### Action identity

ActionId is deterministic from:

- RunId;
- AgentId;
- agent logical sequence;
- model version;
- normalized intent kind.

This makes repeated planning after a crash converge on the same logical action. A unique
database constraint prevents duplicate insertion.

## 5. Deterministic Randomness

At run creation, a master seed is supplied or generated from OS entropy. Per-decision
randomness derives from:

    BLAKE3(master seed || agent id || logical sequence || model version)

The digest seeds a ChaCha RNG. The store persists the logical sequence, not opaque RNG
state. A restart therefore reproduces a pending plan exactly.

Tests pin seed, clock, configuration, and model version. Evidence manifests record them.

## 6. Scheduling

The scheduler uses a min-priority queue keyed by due time and a stable tie-breaker.

Properties:

- O(log n) enqueue and dequeue.
- No sleeping task per agent.
- One active lease per wallet.
- Global and per-protocol concurrency bounds.
- Fairness for agents sharing the same due window.
- Backpressure when the store, RPC, or adapter slows.
- Cancellation stops new claims before in-flight work drains.

Virtual mode advances the clock directly to the next due action. Runtime mode sleeps until
the next due action or a wake-up notification.

## 7. Durable Schema

Planned tables:

### runs

- id, created_at, status;
- master_seed_hash;
- config_hash, model_version, code_revision;
- Surfpool identity and snapshot hash;
- started_at, stopped_at.

### agents

- id, run_id, wallet_alias, public_key;
- persona_json, sequence, status;
- next_due_at;
- budget and observed-balance fields;
- lease_owner, lease_expires_at;
- version for optimistic updates.

### actions

- id, run_id, agent_id, sequence;
- adapter, intent_json, expected_postconditions_json;
- status;
- planned_at, due_at, updated_at;
- idempotency key;
- last_error_class and message.

### attempts

- id, action_id, attempt_number;
- blockhash and validity window;
- serialized transaction hash;
- signature;
- submitted_at, observed_at;
- simulation and confirmation summaries.

Signed bytes may be stored locally for exact retry, but never exported to evidence when
doing so would expose a signer or reusable transaction.

### events

- monotonic id;
- run_id, agent_id, action_id;
- event kind and timestamp;
- versioned public observation payload.

The events table is append-only and feeds the evaluator.

### models and migrations

- model version, config hash, calibration hash;
- schema migration version and applied timestamp.

## 8. Action Lifecycle

Primary path:

    Planned -> Simulated -> Submitted -> Confirmed

Alternative states:

- Rejected: policy or deterministic validation failed.
- Retryable: no chain side effect and a bounded retry is safe.
- Unknown: the response cannot establish whether submission landed.
- Failed: a confirmed chain error.
- Expired: the intent or quote is no longer valid.
- Orphaned: an observed result disappeared after rollback.
- Cancelled: shutdown occurred before submission.

Rules:

1. Planning and policy validation precede transaction construction.
2. Simulation success is required before submission.
3. Prepared transaction hash is stored before send.
4. Signature is stored as soon as it is locally known.
5. Unknown actions enter reconciliation; they are not rebuilt.
6. A blockhash-expired unknown action is classified, then replanned as a new logical
   action only when the original signature is proven absent.
7. Confirmation requires adapter-specific state observation.

## 9. Adapter Contract

Each adapter implements:

- kind and protocol metadata;
- discover current required state;
- quote or derive a bounded action;
- validate protocol/program/mint allowlists;
- prepare a transaction plan;
- declare expected postconditions;
- parse simulation;
- parse confirmation and logs;
- observe state deltas;
- reconcile a known signature;
- classify deterministic and transient errors.

Adapter calls never receive the master seed or ownership labels.

## 10. Policy Ordering

Policy runs in this order:

1. environment and Surfpool identity;
2. action and protocol enabled;
3. signer, program, mint, and destination allowlists;
4. prohibited behavior rules;
5. wallet reserve and rent;
6. per-action, daily, and lifetime budgets;
7. fee, slippage, and price impact;
8. cooldown and sequence restrictions;
9. consolidation and graph constraints;
10. final simulation-derived checks.

A later layer cannot override a rejection from an earlier safety layer.

## 11. Chain Gateway

The gateway is constructed only after:

- RPC and WebSocket URLs parse as loopback;
- getVersion returns surfnet-version;
- surfnet_getSurfnetInfo succeeds;
- the Surfpool network identity is recorded;
- the configured genesis/network identity matches persisted run state.

Only then may the runtime load signer references or claim work.

All blockhashes, account reads, simulations, sends, signature status, and transaction
receipts use this gateway. Off-chain quote APIs may be contacted through adapter-specific
clients, but they never submit transactions.

## 12. Evaluation Boundary

Runtime events contain:

- timestamp and slot;
- public wallet alias or pseudonymous ID;
- action and protocol;
- public amounts and balance fractions;
- signer/fee-payer/funding edges visible to the evaluator;
- result, fees, route, and latency;
- model and run identifiers.

Ground-truth labels contain controller ownership groups and are stored separately.

The evaluator creates:

- feature rows;
- pairwise similarity scores;
- inferred clusters;
- per-attack and composite reports;
- baseline comparisons;
- ablations and seed summaries.

No evaluator result changes runtime behavior in P0. This prevents feedback from silently
optimizing for the exact test set.

## 13. CLI Contract

Planned commands:

- cooker init
- cooker doctor
- cooker plan
- cooker simulate
- cooker run
- cooker status
- cooker recover
- cooker evaluate

run is dry-run unless explicit execution and policy acknowledgement are present. P0 still
rejects non-Surfpool networks even with acknowledgement.

Every command supports JSON output. Secret configuration values are never emitted.

## 14. Scalability Targets

- 1,000 agents for 30 virtual days in one process.
- Bounded tasks independent of fleet size.
- Bounded channel capacities.
- One SQLite writer boundary and WAL readers.
- Configurable global and per-protocol concurrency.
- No full-trace retention in memory during long runs.
- Streaming evaluator input and incremental report aggregates where practical.

The evidence report records wall time, peak memory when available, action count, and
database size. It does not generalize one-machine performance into an unsupported claim
about unlimited scale.

## 15. Future Extension Points

After P0:

- Postgres Store for multiple hosts.
- Remote/KMS signer providers.
- Additional stateful protocol adapters.
- Versioned observed-traffic calibrators.
- Coordinator API and authenticated worker leases.
- More sophisticated attacker models.

These are extension points, not implied first-release capabilities.

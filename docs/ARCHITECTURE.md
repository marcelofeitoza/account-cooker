# Architecture

Status: implemented architecture; canonical release evidence and two independent
clean-clone verifications are complete. PR 2 is open and ready for review.

This repository is a standalone clean-room implementation. It has no source, fixture,
service, key, data, or runtime dependency on any private project; anything conceptually
similar was implemented inside this workspace from public interfaces.

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
                                      verified Solana gateway
                                                  |
                                                  v
                                         receipt observer
                                                  |
                             +--------------------+--------------------+
                             |                                         |
                             v                                         v
                       durable event log                         evaluator trace

The same planner runs against a VirtualClock and a simulated adapter for fast,
deterministic fleet generation. Canonical chain acceptance uses Surfpool. One separate,
ignored, source-declared test uses the native-transfer path against verified public devnet.

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

- AgentId, RunId, ActionId, LeaseId, and deterministic identity derivation.
- Persona and SessionState.
- AgentSnapshot, BudgetUsage, and StateExpectation.
- PlannedAction, PreparedAction, SimulationReceipt, and ChainReceipt.
- ActionKind, ActionPayload, ActionState, ConfirmationStatus, and PolicyDecision.
- BehaviorModel, ActionAdapter, Policy, StateStore, ChainGateway, and Clock contracts.
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

The store is deliberately single-host. The StateStore trait preserves a future path to Postgres
without pretending SQLite provides cross-host coordination.

### cooker-solana

Owns all Solana-specific behavior:

- RpcEndpoint validated type, loopback by default and public-cluster only by explicit
  in-source declaration.
- Surfpool and public-cluster identity and health probes.
- Solana JSON-RPC transport and confirmation polling.
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
- Graceful shutdown and durable lifecycle checkpoints used by the subprocess recovery
  harness.
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
- A global worker bound and one active lease per wallet.
- Fairness for agents sharing the same due window.
- Backpressure when the store, RPC, or adapter slows.
- Cancellation stops new claims before in-flight work drains.

Virtual mode advances the clock directly to the next due action. Runtime mode sleeps until
the next due action or a wake-up notification.

## 7. Durable Schema

Six embedded migrations create `store_identity`, `schema_migrations`, `runs`, `agents`,
`actions`, `leases`, `prepared_transactions`, `simulations`, `submissions`, `receipts`,
`action_events`, `traces`, `action_deferrals`, and `confirmation_audits`.

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

Signed bytes are stored locally for exact observation/reconciliation, but are never
exported to evidence.

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

The default gateway is constructed only after:

- RPC and WebSocket URLs parse as loopback;
- getVersion returns surfnet-version;
- surfnet_getSurfnetInfo succeeds;
- the Surfpool network identity is recorded;
- the configured genesis/network identity matches persisted run state.

Only then may the runtime load signer references or claim work.

A second constructor, `SolanaGateway::connect_public_cluster`, addresses a named public
Solana cluster and is the only path that leaves loopback. It requires an `RpcEndpoint` built
from a `PublicCluster` value written in Rust source, and it proves that cluster's pinned
genesis hash before returning, so a signer is still never loaded against an unverified
network. Application configuration cannot select it: `RpcEndpoint`'s `FromStr` and
`TryFrom<Url>` implementations still accept explicit loopback IPs exclusively, and the
`cooker` CLI never constructs a public endpoint. The dedicated ignored test accepts an
endpoint URL through `COOKER_DEVNET_RPC_URL`, binds it to `PublicCluster::Devnet` in source,
and verifies the pinned devnet genesis hash before loading a signer. It is used by the
bounded devnet native-transfer run documented in
[the devnet run and topology delta](DEVNET.md).

Against a public cluster the same gateway also paces itself under the endpoint's published
request, per-method, and connection ceilings, and retries transient read failures with
bounded exponential backoff. `sendTransaction` is never retried; a failed send stays an
ambiguous outcome that only signature reconciliation may settle.

All blockhashes, account reads, simulations, sends, signature status, and transaction
receipts use this gateway. Off-chain quote APIs may be contacted through adapter-specific
clients, but they never submit transactions.

Canonical Jupiter acceptance does not depend on current pool movement. It verifies a
compressed 21-account Surfpool snapshot and reviewed quote/instruction fixture, removes
the fixture signer, and changes exactly three account identities: the signer and its WSOL
and USDC associated token accounts. The transaction is rebuilt with a current Surfpool
blockhash and a fresh local signer before simulation and submission.

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

No evaluator result changes runtime behavior. This prevents feedback from silently
optimizing for the exact test set.

## 13. CLI Contract

Implemented commands:

- cooker keygen
- cooker fleet-init
- cooker fund
- cooker init
- cooker validate
- cooker plan
- cooker simulate
- cooker evaluate
- cooker soak
- cooker doctor
- cooker run
- cooker status
- cooker recover

`fund`, `run`, and `recover` remain previews unless explicit execution and policy
acknowledgement are present. Non-Surfpool networks are rejected even with acknowledgement.

Every command supports JSON output. Secret configuration values are never emitted.

## 14. Scalability Targets

- 1,000 agents for 30 virtual days in one process.
- Bounded tasks independent of fleet size.
- One SQLite writer boundary and WAL readers.
- Configurable global concurrency with per-wallet exclusivity.
- Per-seed trace materialization followed by atomic JSONL streaming and replay.
- Canonical evidence records action count, wall time, and peak RSS rather than implying
  unbounded scale.

The evidence report records wall time, peak memory when available, action count, and
database size. It does not generalize one-machine performance into an unsupported claim
about unlimited scale.

## 15. Deliberately Unimplemented Extensions

- Postgres Store for multiple hosts.
- Remote/KMS signer providers.
- Additional stateful protocol adapters.
- Marinade staking or unstaking. The shipped stateful path is native Solana stake and is
  not represented as a Marinade integration.
- Versioned observed-traffic calibrators.
- Coordinator API and authenticated worker leases.
- More sophisticated attacker models.

These are extension points, not implied capabilities or completion gates for this bounty
deliverable.

## 16. Architecture Limits

- Coordination and durability are single-host: one controller, local signer files, and
  one SQLite WAL database. The worker bound scales local concurrency; it is not a
  distributed execution claim.
- Acceptance-scale chain execution is local Surfpool. One bounded public devnet run tests
  public-network inclusion and durability; see [the devnet run and topology delta](DEVNET.md)
  for what it does and does not establish. Neither harness protects RPC/IP metadata.
- The Jupiter fixture is an offline snapshot captured from a lazy fork at slot `433717382`;
  it proves the reviewed route and signer-rebinding contract, not current market state.
- Evaluator inputs and ownership labels are synthetic known ground truth. They validate
  the declared attacker implementation but do not model every proprietary analyst or the
  full distribution of human Solana behavior.
- A shared fleet funder remains directly visible in the transaction graph.

# Threat Model And Claim Policy

Status: implemented claim contract with committed canonical comparative evidence. No
anonymity, unlinkability, or human-indistinguishability result is claimed.

## 1. Purpose

Account Cooker attempts to reduce selected behavioral signals that analysts use to group
wallets controlled by one operator. It does not hide transaction contents, provide
cryptographic anonymity, or erase historical funding relationships.

The primary failure mode is not a broken signature or stolen key. It is a privacy claim
that exceeds what the generated traces and adversarial tests establish.

## 2. Protected Properties

The system may attempt to reduce confidence in:

- behavioral ownership clustering across a fleet;
- repeated cadence and active-hour fingerprints;
- repeated amount and roundness fingerprints;
- protocol-sequence and session-shape fingerprints;
- deterministic consolidation patterns;
- operator-induced synchrony.

It also protects operational integrity:

- no duplicate logical spend after restart;
- budgets and reserves remain enforced;
- generated keys do not leak;
- chain configuration cannot redirect execution to a public network, and the one
  public-cluster path is declared in source, opt-in, devnet-pinned by genesis hash, and
  excluded from the canonical run;
- evidence cannot be confused with organic-user traffic.

## 3. Adversaries

### Public post-hoc observer

Reads confirmed Solana transactions, accounts, token balances, program calls, slots, fees,
and historical graph relationships.

### Infrastructure observer

May observe RPC timing, client network metadata, or pre-inclusion flow. The evaluator does not model
or defend network-layer correlation.

On loopback this adversary is hypothetical, because nothing sits between the controller and
the chain. The bounded devnet run sent every request through one shared RPC provider from one
source address. That provider could observe request arrival timing and client network
metadata before inclusion. Account Cooker does not defend that correlation channel; see
[the devnet run and topology delta](DEVNET.md).

### Statistical clusterer

Uses timing, amounts, action sequences, funding edges, fee payers, balances, routes, and
destinations to infer common ownership.

### Copy trader

Attempts to infer a strategy from repeated public actions. Account Cooker can add behavior
diversity but cannot hide the actual executed instruction or resulting position.

### Faulty or malicious operator configuration

Attempts to exceed budget, use a public RPC, enable a prohibited adapter, leak secrets, or
mislabel generated traffic.

## 4. Observed Leakage Channels

| Channel | Visible signal | Implemented treatment |
|---|---|---|
| funding | common ancestor, star fan-out | measured per payer-assignment scheme; see section 7 |
| fee payer | reused signer or payer | measured and policy-constrained |
| timing | cadence, time zone, bursts | persona sessions and evaluator |
| amounts | exact values, roundness, ratios | bounded mixture and evaluator |
| sequence | repeated protocol transitions | conditional model and evaluator |
| synchrony | wallets acting together | global scheduler and evaluator |
| consolidation | common fan-in and delay | explicit lifecycle and policy |
| destination | repeated counterparties | measured; restricted by policy |
| transaction data | programs, accounts, amounts | not hidden |
| network metadata | IP/RPC connection timing | out of scope |
| external identity | KYC, exchange, public disclosures | out of scope |

## 5. Non-Claims

The project does not claim:

- anonymity;
- cryptographic unlinkability;
- hiding of funds or transaction contents;
- resistance to a global passive observer;
- resistance to RPC or IP correlation;
- removal of the funding graph itself, as opposed to the measured removal of the
  operator-adjacency signal inside it described in section 7;
- statistical indistinguishability from all human users;
- protection from subpoenas, KYC records, or off-chain identity;
- legal or protocol permission for automated mainnet activity;
- that more transactions always improve privacy.

## 6. Measurement Contract

Every privacy comparison declares:

- population and controller grouping;
- action and budget equality;
- baseline behavior;
- proposed model version;
- attacker features;
- threshold selection method;
- seeds and splits;
- metrics;
- blind spots.

Controller labels cannot enter feature extraction. Thresholds are fixed before held-out
evaluation. Reports include all seeds and failed ablations, not only the best result.

Synthetic correctness and privacy effectiveness are separate:

- Synthetic ground truth establishes whether metrics behave correctly.
- Generated Surfpool traces establish chain fidelity.
- Neither establishes resemblance to the full population of human Solana users.

## 7. Funding-Provenance Bound

If all wallets receive funds from a labeled operator wallet, the funding graph directly
links them. Timing and action diversity cannot undo that historical edge. This is the
baseline condition, and the evaluator measures it at ROC AUC 1.0000 on every seed and
every planner, under all three funding attacks.

The evaluator also measures what changes that. Three payer-assignment schemes run over one
shared participation schedule, with identical behavior, budgets, attacker, and threshold,
so the payer assignment is the only difference between them:

- `dedicated_per_operator`, the baseline star topology above;
- `pooled_mixed_rounds`, a shared disburser set paying one uniform denomination per
  transfer, with each round's recipients shuffled together before disbursers are dealt out;
- `pooled_per_operator_rounds`, the control, which keeps the shared disburser set and the
  uniform denomination but lets each operator's batch go to a single disburser.

The pooled mixed scheme is built so that no operator grouping can reach the payer
assignment. Its schedule is a function of account identifiers and a schedule seed only,
which a unit test enforces by relabelling every operator and requiring the same plan. That
is why the drop is a structural property and not a tuned parameter.

The implementation therefore:

- measures funding provenance per scheme rather than conceding it;
- attacks each scheme with common-funder overlap, payer-and-round overlap, and a
  scheme-aware attack that weights a shared batch by how few accounts were in it;
- reports composite results both with and without funding features;
- keeps the control arm, which shows that a shared pool alone does not close the channel;
- refuses to describe such a fleet as unlinkable;
- does not create deceptive multi-hop fan-out and call it privacy.

What the pooled result does not establish: deposits into the pool, custody of the pool, the
fleet-level fact that every account appears in the first round, and value or count matching
across the pool boundary are all outside the observation model. A batch with one recipient
gives that transfer no cover, and the smallest-batch column reports it.

The scope is the evaluator. `cooker-core::funding` builds the schedules and `cooker-eval`
measures them. The runtime `cooker fund` path is unchanged and still pays each account from
the operator wallet. No pool, mixer, or custodial service is implemented, and any real
funding backend requires its own threat model and approval.

## 8. Transaction Transparency Bound

Solana exposes account keys, programs, instruction data, signatures, balances, and state
changes. Decoy no-ops or unusual routing may increase distinguishability.

Account Cooker changes scheduling and real protocol use. It does not claim to conceal the
content of those actions.

## 9. Abuse Controls

The implementation excludes:

- self-trades and reciprocal fleet trades;
- volume loops intended to inflate metrics;
- governance voting;
- NFT bidding or floor manipulation;
- referral or rewards farming;
- Sybil participation in airdrops;
- dust spam;
- bridge churn;
- actions against unapproved programs, mints, or destinations.

Controls:

- loopback-only endpoint validation on every configuration-reachable path;
- dry-run default;
- explicit execution acknowledgement;
- protocol, program, mint, signer, and destination allowlists;
- principal, fee, loss, slippage, and action-count budgets;
- cooldowns and minimum reserves;
- kill switch and graceful shutdown;
- generated-traffic attribution in local records;
- no public-network execution path through the CLI, configuration, or canonical run, and
  no mainnet path at all.

The single public-cluster path is `RpcEndpoint::public_cluster` plus
`SolanaGateway::connect_public_cluster`, used by the opt-in bounded devnet native-transfer
run. It requires
a `PublicCluster` value written in Rust source, the only such value is devnet, and the
gateway proves devnet's pinned genesis hash before a signer is loaded. See
[the devnet run and topology delta](DEVNET.md).

## 10. Operational Threats

### Duplicate submission

Mitigation: deterministic ActionId, unique constraint, persisted prepared transaction,
stored signature, and mandatory unknown-outcome reconciliation.

The recovery acceptance test pauses real child processes at six durable lifecycle
boundaries, flushes a marker, sends `SIGKILL`, and recovers against the same SQLite database
and Surfpool. This tests process loss without graceful cleanup; it does not simulate a
whole-host disk failure.

### Stale blockhash

Mitigation: validity tracking and safe replan only after the prior transaction is proven
absent. Unknown transactions are never rebuilt automatically.

### RPC redirection

Mitigation: typed loopback URL plus Surfpool-specific identity probes before signer/store
runtime activation. The devnet path substitutes an equally strong probe, a pinned genesis
hash, and is unreachable from configuration.

### Key leakage

Mitigation: ephemeral files under an ignored directory, mode 0600, redacted config/logs,
and no key material in evidence.

### Database corruption or stale Surfnet

Mitigation: migrations, WAL, integrity checks, run/network identity binding, and refusal to
resume against a different Surfnet state.

### Misleading evidence

Mitigation: manifest hashes, fixed seeds, version capture, command transcript, pre/post
state assertions, and explicit separation of generated and reference data.

Canonical results are not claimed until `evidence/final` exists for a clean commit and the
full gate has passed twice from fresh clones and fresh Surfpool state.

## 11. Claim Language

Permitted before favorable evaluation:

- generates policy-constrained account workloads;
- provides deterministic simulation and Surfpool execution;
- measures selected wallet-clustering signals;
- reports common-funder and longitudinal leakage;
- reports funding provenance per payer-assignment scheme against a named baseline.

Permitted only after evidence:

- reduced a named attacker metric relative to a named baseline by a reported amount;
- preserved a named budget/recovery invariant in a reported Surfpool scenario;
- executed a reported number of agents/actions under a stated configuration.

Never permitted:

- anonymous;
- untraceable;
- impossible to cluster;
- statistically indistinguishable without a named population and test;
- defeats modern chain analysis as a universal statement.

## 12. Residual Risk

Even a favorable report can miss proprietary analytics, network metadata, cross-chain
identity, external service records, adaptive adversaries, and distribution shift.

Additional fixed limitations are:

- a common fleet funder remains directly observable;
- attacker traces and controller labels are synthetic known ground truth, not a sample of
  the complete human Solana population;
- deterministic Jupiter proof uses a route-specific offline snapshot captured from a lazy
  fork at slot `433717382`, not current market state;
- coordination is one local controller, local signer files, SQLite, and loopback Surfpool,
  not a distributed topology; one bounded devnet run adds a public-network data point but is
  not a sustained-load result, and devnet is not mainnet;
- the evaluator has never been run on public-network traces, so its privacy metrics are not
  established against real inclusion delay, slot granularity, or transaction loss;
- the stateful protocol path is native Solana stake, not Marinade.

The contribution is a workload and measurement tool, not a guarantee of privacy.

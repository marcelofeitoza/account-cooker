# Threat Model And Claim Policy

Status: design contract; no privacy result is claimed.

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
- chain configuration cannot redirect execution to a public network;
- evidence cannot be confused with organic-user traffic.

## 3. Adversaries

### Public post-hoc observer

Reads confirmed Solana transactions, accounts, token balances, program calls, slots, fees,
and historical graph relationships.

### Infrastructure observer

May observe RPC timing, client network metadata, or pre-inclusion flow. P0 does not model
or defend network-layer correlation.

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

| Channel | Visible signal | P0 treatment |
|---|---|---|
| funding | common ancestor, star fan-out | measured; not claimed solved |
| fee payer | reused signer or payer | measured and policy-constrained |
| timing | cadence, time zone, bursts | persona sessions and evaluator |
| amounts | exact values, roundness, ratios | bounded mixture and evaluator |
| sequence | repeated protocol transitions | conditional model and evaluator |
| synchrony | wallets acting together | global scheduler and evaluator |
| consolidation | common fan-in and delay | explicit lifecycle and policy |
| destination | repeated counterparties | measured; restricted in safe P0 |
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
- removal of a common-funder graph;
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

## 7. Common-Funder Bound

If all wallets receive funds from a labeled operator wallet, the funding graph directly
links them. Timing and action diversity cannot undo that historical edge.

P0 therefore:

- measures common funding explicitly;
- reports results both with and without funding features;
- refuses to describe such a fleet as unlinkable;
- accepts pre-funded test wallets only as a controlled experimental condition;
- does not create deceptive multi-hop fan-out and call it privacy.

Any future funding backend requires its own threat model and approval.

## 8. Transaction Transparency Bound

Solana exposes account keys, programs, instruction data, signatures, balances, and state
changes. Decoy no-ops or unusual routing may increase distinguishability.

Account Cooker changes scheduling and real protocol use. It does not claim to conceal the
content of those actions.

## 9. Abuse Controls

P0 excludes:

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

- Surfpool-only endpoint validation;
- dry-run default;
- explicit execution acknowledgement;
- protocol, program, mint, signer, and destination allowlists;
- principal, fee, loss, slippage, and action-count budgets;
- cooldowns and minimum reserves;
- kill switch and graceful shutdown;
- generated-traffic attribution in local records;
- no public-network execution code path in P0.

## 10. Operational Threats

### Duplicate submission

Mitigation: deterministic ActionId, unique constraint, persisted prepared transaction,
stored signature, and mandatory unknown-outcome reconciliation.

### Stale blockhash

Mitigation: validity tracking and safe replan only after the prior transaction is proven
absent. Unknown transactions are never rebuilt automatically.

### RPC redirection

Mitigation: typed loopback URL plus Surfpool-specific identity probes before signer/store
runtime activation.

### Key leakage

Mitigation: ephemeral files under an ignored directory, mode 0600, redacted config/logs,
and no key material in evidence.

### Database corruption or stale Surfnet

Mitigation: migrations, WAL, integrity checks, run/network identity binding, and refusal to
resume against a different Surfnet state.

### Misleading evidence

Mitigation: manifest hashes, fixed seeds, version capture, command transcript, pre/post
state assertions, and explicit separation of generated and reference data.

## 11. Claim Language

Permitted before favorable evaluation:

- generates policy-constrained account workloads;
- provides deterministic simulation and Surfpool execution;
- measures selected wallet-clustering signals;
- reports common-funder and longitudinal leakage.

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

The contribution is a workload and measurement tool, not a guarantee of privacy.

# Account Cooker

Account Cooker is a standalone Rust engine for generating durable, policy-constrained
Solana account workloads and measuring the behavioral signals those workloads expose to
wallet-clustering attacks. It treats privacy as an empirical systems problem: the runtime
must survive faults without duplicate execution, and the evaluator must report failed
privacy hypotheses as plainly as successful ones.

> Status: implementation, canonical evidence, two independent clean-clone release
> verifications, published-policy eligibility audit, and PR handoff are complete.
> Upstream PR 2 is open and ready for review.

## What Ships

- Deterministic persona and semi-Markov session planning with one bounded priority
  scheduler; canonical evidence covers a 1,000-agent fleet.
- SQLite WAL persistence with migrations, atomic leases, deterministic action IDs,
  immutable journals, persisted signed transactions, historical confirmation audits, and
  unknown-outcome reconciliation.
- A real subprocess recovery matrix that pauses at six durable lifecycle boundaries,
  force-terminates each child with `SIGKILL`, reopens the same SQLite database, and proves
  exact submit, signature, event, and balance outcomes against Surfpool.
- A fail-closed Surfpool gateway and local signer boundary. Network validation happens
  before a signer is loaded or mutable runtime state is opened.
- Locally signed native SOL, classic SPL, Jupiter exact-input, and native stake lifecycle
  adapters with simulation, confirmation, and protocol-specific postconditions.
- Dry-run-first fleet funding, planning, execution, status, recovery, and kill-switch
  controls through the `cooker` CLI.
- A label-isolated adversarial evaluator covering timing, amounts, action sequences,
  synchrony, destinations, balance rank, consolidation, route, fee-payer, and funding
  signals across five fixed seeds and feature ablations.
- Three funding-provenance schemes measured under identical behavior, budgets, and
  attacker settings, including a scheme-aware attack that weights small shared batches.
- Deterministic virtual and real Surfpool soak harnesses plus a sanitized,
  checksum-bearing evidence pack.
- One opt-in bounded public devnet run with committed, explorer-checkable transaction
  signatures, and a written statement of what that run establishes and leaves unmeasured.

## Claim Boundary

Account Cooker does not provide anonymity, cryptographic unlinkability, transaction
confidentiality, or protection from RPC/IP correlation. The evaluator measures named
synthetic attacker models; it does not establish resemblance to the full population of
human Solana users.

Funding provenance is now measured rather than conceded. A dedicated per-operator funding
wallet is perfectly linkable: all three funding attacks score ROC AUC 1.0000 on every seed
and every planner. Paying the same accounts from a shared pool that uses one uniform
denomination and mixes each round's recipients across operators drops the same three
attacks to chance, and the same pool batched per operator does not, which is what isolates
the mixing rather than the pool as the cause. The measurement covers the disbursement side
only; deposits into the pool, custody of the pool, and value or count matching across the
pool boundary are unmeasured, so this does not establish transaction-graph anonymity.

That result is an evaluator measurement, not a shipped capability. The runtime `cooker fund`
command is unchanged and still pays each account from the operator wallet. No pool, mixer,
or custodial service is implemented. See
[the funding-provenance measurement](evidence/funding/README.md).

The project does not implement self-trading, artificial volume, governance voting, referral
or airdrop farming, NFT manipulation, dust spam, bridge churn, or deceptive multi-hop
funding.

Chain execution is loopback Surfpool by default and by configuration. There is exactly one
public-network path, an opt-in bounded devnet test that must be started by hand and that
names its cluster in Rust source. Application configuration and CLI flags cannot select a
public cluster. The dedicated ignored harness accepts an endpoint URL override, but hardcodes
devnet and verifies its pinned genesis hash before loading a signer. The canonical
`scripts/full-demo.sh` run never sends off loopback. Every transaction in the canonical
evidence pack is loopback. The separate devnet record and comparison are in
[the devnet run and topology delta](docs/DEVNET.md). There is no mainnet execution path.

The current topology is one local controller, one SQLite store, local signer files, and one
loopback Surfpool process, plus that single bounded devnet run against one public RPC
endpoint. A bounded run is not a sustained-load proof and devnet is not mainnet. The
stateful adapter is native Solana stake, not Marinade. Deterministic Jupiter evidence uses a
reviewed snapshot captured from a lazy fork at a fixed slot; its age and limited account set
make it reproducible, not representative of current market state.

## Reproduce It

The pinned environment is:

- Rust `1.92.0`;
- Surfpool `1.4.0`;
- Agave CLI `3.1.8` for `solana-keygen`;
- `bash`, `curl`, `gzip`, `jq`, `lsof`, and `shellcheck`;
- `cargo-audit 0.22.1`, `cargo-deny 0.20.2`, and Gitleaks `8.30.1` for the complete gate.

Two safety postures are declared where a reviewer can see them in one file. All 20 Cargo
target roots carry an explicit `#![forbid(unsafe_code)]` on top of the workspace-wide
`unsafe_code = "forbid"` lint, and the RustSec advisory policy is checked in at
[`.cargo/audit.toml`](.cargo/audit.toml), which denies unmaintained, unsound, and yanked
dependencies and records a reason for each accepted exception. `cargo audit` reads that
file directly, and `./scripts/check-lint-policy.sh` fails if either posture is dropped.
Both run in CI. See [validation and evidence gates](docs/VALIDATION.md).

The harness starts Surfpool offline from that pinned snapshot with instruction profiling
disabled. Remote account misses and profiling are not part of the acceptance claim and
would otherwise add external rate limits or unrelated resource work to canonical soak
execution.

Run the reduced rehearsal while developing:

```bash
./scripts/full-demo.sh --quick
```

Run the canonical proof from a clean Git tree:

```bash
./scripts/full-demo.sh
```

The canonical command runs all Rust and shell quality gates, supply-chain and secret
scans, a release build, the full CLI lifecycle, the five-seed evaluator, a deterministic
1,000-agent by 30-day virtual soak, a faulted 1,000-transaction Surfpool soak, and six
acceptance groups: native SOL, reviewed-state Jupiter, classic SPL, fault reconciliation,
the six-checkpoint process-crash matrix, and native stake. Stake runs last because it
advances Surfpool across epochs. The demo uses fresh, isolated, offline Surfpool state; no
chain RPC request leaves loopback and no transaction is written to a public RPC.

The public devnet native-transfer run is deliberately not part of that command. It spends
real devnet SOL,
so it is opt-in, run by hand, and recorded separately:

```bash
./scripts/devnet-soak.sh --transactions 200
```

Read [the devnet run and topology delta](docs/DEVNET.md) before citing either result. It
states what the public run establishes and what remains unmeasured.

## CLI

```text
cooker keygen       Generate a permission-checked local Surfpool signer
cooker fleet-init   Create a durable multi-signer fleet
cooker fund         Preview or execute idempotent local fleet funding
cooker init         Write the documented configuration
cooker validate     Validate configuration without network access
cooker plan         Preview deterministic observable actions
cooker simulate     Generate stable trace JSONL
cooker evaluate     Run the comparative adversarial experiment
cooker soak         Run and replay a virtual soak
cooker doctor       Probe Surfpool without loading a signer
cooker run          Preview or execute a bounded fleet pass
cooker status       Inspect SQLite state without network or signer access
cooker recover      Audit and reconcile without blind resubmission
```

`fund`, `run`, and `recover` are previews unless both `--execute` and
`--acknowledge-policy` are supplied. `doctor`, previews, and `status` report that no signer
was loaded and whether state changed, making the safety boundary machine-verifiable.

## Architecture

| Crate | Ownership |
|---|---|
| `cooker-core` | domain types, personas, deterministic behavior, scheduler, safety policy |
| `cooker-store` | SQLite migrations, leases, journals, budgets, audits, reconciliation |
| `cooker-solana` | verified Surfpool or declared-devnet RPC, local signers, transaction lifecycle, four adapters |
| `cooker-runtime` | bounded fleet workers, recovery, confirmation auditing, shutdown |
| `cooker-eval` | label-free features, transparent attacks, metrics, ablations, reports |
| `cooker-cli` | configuration, operator commands, virtual soak, evidence-facing output |

The implementation is clean-room MIT code based on the bounty specification and public
Solana, Surfpool, Jupiter, Raydium, and SPL interfaces. It is standalone and has no code,
service, data, fixture, key, or runtime dependency on any private project.

## Evidence

Raw databases, keys, local Surfpool signatures, and logs stay under ignored directories. The
evidence builder validates them and exports only redacted structured results, exact tool and
snapshot hashes, supported metrics, commands, and checksums. The separate devnet record
intentionally commits public signatures so third parties can check them. See
[the evidence guide](evidence/README.md) and [validation contract](docs/VALIDATION.md).

## Design Documents

- [Master implementation contract](PLAN.md)
- [Architecture](docs/ARCHITECTURE.md)
- [Threat model and claim policy](docs/THREAT_MODEL.md)
- [Surfpool runbook](docs/SURFPOOL.md)
- [Public devnet run and topology delta](docs/DEVNET.md)
- [Validation and evidence gates](docs/VALIDATION.md)
- [Bounty eligibility audit](docs/ELIGIBILITY.md)

This repository targets Superteam Brasil's
[Privacy-Through-Noise tooling bounty](https://superteam.fun/earn/listing/noise), in the
`account-cooker` track.

## License

MIT. See [LICENSE](LICENSE).

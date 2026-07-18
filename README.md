# Account Cooker

Account Cooker is a standalone Rust engine for generating durable, policy-constrained
Solana account workloads and measuring the behavioral signals those workloads expose to
wallet-clustering attacks. It treats privacy as an empirical systems problem: the runtime
must survive faults without duplicate execution, and the evaluator must report failed
privacy hypotheses as plainly as successful ones.

> Status: the complete implementation passes the reduced end-to-end rehearsal. The
> canonical clean-tree evidence run and two fresh-clone verification runs are the remaining
> engineering release gates. The upstream pull request remains a draft.

## What Ships

- Deterministic persona and semi-Markov session planning with one bounded priority
  scheduler for fleets of thousands of agents.
- SQLite WAL persistence with migrations, atomic leases, deterministic action IDs,
  immutable journals, persisted signed transactions, historical confirmation audits, and
  unknown-outcome reconciliation.
- A fail-closed Surfpool gateway and local signer boundary. Network validation happens
  before a signer is loaded or mutable runtime state is opened.
- Locally signed native SOL, classic SPL, Jupiter exact-input, and native stake lifecycle
  adapters with simulation, confirmation, and protocol-specific postconditions.
- Dry-run-first fleet funding, planning, execution, status, recovery, and kill-switch
  controls through the `cooker` CLI.
- A label-isolated adversarial evaluator covering timing, amounts, action sequences,
  synchrony, destinations, balance rank, consolidation, route, fee-payer, and funding
  signals across five fixed seeds and feature ablations.
- Deterministic virtual and real Surfpool soak harnesses plus a sanitized,
  checksum-bearing evidence pack.

## Claim Boundary

Account Cooker does not provide anonymity, cryptographic unlinkability, transaction
confidentiality, or protection from RPC/IP correlation. A common funding edge remains
directly observable. The evaluator measures named synthetic attacker models; it does not
establish resemblance to the full population of human Solana users.

The project also does not implement public-network execution, self-trading, artificial
volume, governance voting, referral or airdrop farming, NFT manipulation, dust spam,
bridge churn, or deceptive multi-hop funding. Every chain transaction in development and
evidence runs executes only inside loopback Surfpool.

## Reproduce It

The pinned environment is:

- Rust `1.92.0`;
- Surfpool `1.4.0`;
- Agave CLI `3.1.8` for `solana-keygen`;
- `bash`, `curl`, `gzip`, `jq`, `lsof`, and `shellcheck`;
- `cargo-audit 0.22.1`, `cargo-deny 0.20.2`, and Gitleaks for the complete gate.

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
1,000-agent by 30-day virtual soak, a faulted 1,000-transaction Surfpool soak, and five
adapter/fault acceptance scenarios. It starts and stops only its own isolated Surfpool
process and never writes to a public RPC.

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
| `cooker-solana` | Surfpool RPC, local signers, transaction lifecycle, four adapters |
| `cooker-runtime` | bounded fleet workers, recovery, confirmation auditing, shutdown |
| `cooker-eval` | label-free features, transparent attacks, metrics, ablations, reports |
| `cooker-cli` | configuration, operator commands, virtual soak, evidence-facing output |

The implementation is clean-room MIT code based on the bounty specification and public
Solana, Surfpool, Jupiter, Raydium, and SPL interfaces. It has no code, service, data,
fixture, key, or runtime dependency on another private project.

## Evidence

Raw databases, keys, full signatures, and logs stay under ignored directories. The
evidence builder validates them and exports only redacted structured results, exact tool
and snapshot hashes, supported metrics, commands, and checksums. See
[the evidence guide](evidence/README.md) and [validation contract](docs/VALIDATION.md).

## Design Documents

- [Master implementation contract](PLAN.md)
- [Architecture](docs/ARCHITECTURE.md)
- [Threat model and claim policy](docs/THREAT_MODEL.md)
- [Surfpool runbook](docs/SURFPOOL.md)
- [Validation and evidence gates](docs/VALIDATION.md)

This repository targets Superteam Brasil's
[Privacy-Through-Noise tooling bounty](https://superteam.fun/earn/listing/noise), in the
`account-cooker` track.

## License

MIT. See [LICENSE](LICENSE).

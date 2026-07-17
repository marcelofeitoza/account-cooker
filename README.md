# Account Cooker

Account Cooker is a planned Rust system for generating long-lived, policy-constrained
Solana account activity and measuring how well that activity resists behavioral
clustering. The project treats privacy as an empirical systems problem: every behavior
model must be evaluated against explicit attacker features and reproducible evidence.

> Status: planning scaffold. No runtime or privacy result is implemented or claimed yet.

## Target Outcome

The first complete version will provide:

- A deterministic persona and session model for thousands of virtual agents.
- A persistent scheduler with idempotency, restart recovery, budgets, and a kill switch.
- Native transfer, SPL transfer, Jupiter swap, and one stateful protocol adapter.
- A Rust evaluator for funding, timing, amount, cadence, sequence, and consolidation leaks.
- Reproducible Surfpool scenarios, integration tests, accelerated soaks, and evidence packs.

## Locked Principles

1. **Surfpool for all development.** Every chain-facing adapter, recovery path, and soak
   must run against a reproducible Surfpool network. Development code never writes to
   mainnet or devnet.
2. **Measured claims only.** Random timing is not assumed to create privacy. Reports must
   compare a declared baseline, held-out traces, multiple seeds, and explicit metrics.
3. **Safe defaults.** The first release excludes governance voting, self-trading, referral
   farming, artificial volume, NFT bidding, dust spam, and uncontrolled mainnet execution.
4. **Clean-room implementation.** The implementation is new MIT-licensed Rust code based
   on the bounty specification and public interfaces.
5. **Honest boundary.** Behavioral cover traffic cannot erase a common funding graph or
   hide data that Solana transactions publish. Those channels are measured and reported.

## Planned Workspace

```text
crates/
  cooker-core/       Domain types, personas, scheduling, policies
  cooker-store/      SQLite state, leases, idempotency, recovery
  cooker-solana/     RPC, signer, transaction, and adapter implementations
  cooker-runtime/    Controller, workers, reconciliation, telemetry
  cooker-eval/       Trace schema, adversary features, metrics, reports
  cooker-cli/        init, plan, run, status, recover, evaluate
scripts/
  surfpool-start.sh  Canonical local network bootstrap
  full-demo.sh       Clean-state build, run, recovery, and evaluation proof
tests/
  fixtures/          Synthetic ground truth and sanitized feature fixtures
  surfpool/          Chain-facing integration scenarios
evidence/            Small, reproducible proof manifests and reports
```

The final crate boundaries may be collapsed where doing so makes the one-day build more
coherent. Interface ownership and acceptance behavior are fixed in the design documents.

## Design Documents

- [Master implementation plan](PLAN.md)
- [Architecture](docs/ARCHITECTURE.md)
- [Threat model and claim policy](docs/THREAT_MODEL.md)
- [Surfpool development contract](docs/SURFPOOL.md)
- [Validation and evidence gates](docs/VALIDATION.md)

## Bounty

This repository is being developed for Superteam Brasil's
[Privacy-Through-Noise tooling bounty](https://superteam.fun/earn/listing/noise),
in the `account-cooker` track.

## License

MIT. See [LICENSE](LICENSE).

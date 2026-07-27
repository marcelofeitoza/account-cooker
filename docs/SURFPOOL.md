# Surfpool Development Contract

Status: implemented. The expanded six-group acceptance matrix and canonical full-demo are
release gates; no canonical evidence pack is claimed yet.

Surfpool is mandatory for every chain-facing development, integration, recovery, and soak
workflow in Account Cooker. Application transactions are locally signed and submitted to
loopback Surfpool only. The harness uses the mainnet feature baseline in offline mode and
loads every non-native account it needs from the pinned reviewed snapshot.

## 1. Pinned Environment

`configs/surfpool.env` is the executable source of truth:

- Surfpool CLI `1.4.0`;
- Darwin arm64 binary SHA-256
  `11937d7655bc3a04dd90ae6e6038b9b9db59f2e0d48e1bed35b214fcae3dea24`;
- Linux x86_64 binary SHA-256
  `49907282c15e0d9a523c796fc150f3d784e53a68273e998916ee07cac24b59f0`;
- interactive RPC/WebSocket defaults `127.0.0.1:8899` and `127.0.0.1:8900`;
- isolated full-demo defaults `127.0.0.1:18899` and `127.0.0.1:18900`;
- one trillion local airdrop lamports on a fresh persistent database;
- maximum 2,000 Surfpool profiles.

The harness refuses an unsupported platform, wrong version, wrong binary digest,
non-loopback host, malformed port, occupied port, or conflicting port assignment.

## 2. Lifecycle

Start, verify, and stop the default local network:

```bash
./scripts/surfpool-start.sh
./scripts/surfpool-doctor.sh
./scripts/surfpool-stop.sh
```

The start script:

1. verifies the pinned binary and snapshot digests;
2. creates mode-0700 ignored key/state/log directories;
3. generates or validates a mode-0600 funder under `.surfpool/keys`;
4. refuses to attach to an unowned process or occupied port;
5. starts Surfpool with no deploy, TUI, or Studio surface;
6. waits for `getVersion`, requires the exact `surfnet-version`, and calls
   `surfnet_getSurfnetInfo`;
7. writes a mode-0600 runtime environment and session provenance record.

The stop script signals only the PID recorded by this harness after verifying that the
process command is the pinned Surfpool binary. It never uses a broad process-kill command.

## 3. Fail-Closed Application Guard

Every configuration-reachable path builds an `RpcEndpoint` through `FromStr` or
`TryFrom<Url>`, which accept only plain-HTTP URLs whose host is an explicit IPv4 or IPv6
loopback address with a port. `SolanaGateway::connect` then requires Surfpool `1.4.0` and a
valid Surfnet-info response.

Online CLI commands perform this preflight before loading a signer. Preview, `status`, and
configuration validation perform no network request and load no signer. Runtime state is
bound to its configured Surfnet identity; a mismatched store or fleet manifest is rejected.

The one path that addresses a public cluster, `RpcEndpoint::public_cluster` plus
`SolanaGateway::connect_public_cluster`, requires a `PublicCluster` value written in Rust
source and proves that cluster's pinned genesis hash before returning. No configuration
file, environment variable, or CLI flag can reach it, and `scripts/full-demo.sh` never runs
it. See [the devnet run and topology delta](DEVNET.md).

The following fail before transaction construction:

- mainnet, devnet, testnet, or any other non-loopback RPC URL supplied as configuration;
- a local validator that does not expose Surfpool identity RPCs;
- a Surfpool version other than `1.4.0`;
- a durable store or fleet manifest bound to another Surfnet;
- a signer outside the project root's `.surfpool/keys` or `.devnet/keys` directories.

## 4. Reviewed Jupiter State

Deterministic Jupiter acceptance uses:

```text
fixtures/surfpool/jupiter/
  raydium-clmm-sol-usdc-433717382.snapshot.json.gz
  raydium-clmm-sol-usdc-433717382.snapshot.manifest.json
  raydium-clmm-sol-usdc-cyb-433411234.quote.json
  raydium-clmm-sol-usdc-cyb-433411234.instructions.json
```

The archive contains 21 public accounts from a Surfpool `preTransaction` export at slot
`433717382`. It includes the Raydium CLMM/Jupiter program state and lookup table needed for
the reviewed SOL-to-USDC route. The fixture signer account was removed, and no private key
or signed transaction is present.

The harness verifies:

- archive SHA-256
  `f1efd977ec801baec8856e594d215965414440a8a0158b7fcc607b1bb196f2b1`;
- decompressed SHA-256
  `b2c428d69c90388c3dcdfa91f1789c82a6584544885eb6272b55cbb7094aeaab`.

At runtime the acceptance test changes only three reviewed identities in the instruction
account metas: the fixture signer, its WSOL ATA, and its USDC ATA. A non-network invariant
test reverses those substitutions and proves the complete instruction response is
otherwise byte-equivalent. The new transaction uses a current Surfpool blockhash and the
fresh local signer, then simulates, submits, confirms, and checks exact token deltas.

Live Jupiter quote/instruction retrieval remains an explicitly selected planning mode. It
never receives a private key or signed transaction, and the same mint, amount, slippage,
program, account, blockhash, simulation, and postcondition checks apply. Canonical evidence
uses the reviewed fixture so current pool movement cannot invalidate reproducibility.

## 5. Persistent Restart Contract

Every persistent Surfpool database has a companion session record containing:

- binary version and digest;
- network, Surfnet ID, RPC/WebSocket endpoints, and database path;
- snapshot archive and decompressed digests;
- configured and effective airdrop amounts;
- the mandatory instruction-profiling-disabled setting;
- the mandatory offline-datasource setting;
- whether this start resumed an existing database.

On restart, the harness requires exact provenance equality. It sets the effective airdrop
to zero so the funder's balance is not reset while destination accounts retain prior
state. A database without its key/session, a session without its database, or any identity
mismatch fails closed.

Instruction profiling is disabled in the exact recorded start command. The canonical soak
measures transaction durability and state, not Surfpool's optional instruction profiler;
keeping it enabled would add an unrelated CPU and memory workload to the 1,000-transaction
gate.

Offline mode is also part of that exact command and session contract. This prevents unique
soak destinations from becoming lazy remote account fetches, removes external provider rate
limits from the result, and makes missing-account behavior deterministic.

## 6. Chain Acceptance

Run the adapter and fault matrix against an isolated Surfpool:

```bash
./scripts/surfpool-chain-acceptance.sh
```

It executes and records sanitized structured evidence for:

1. native SOL transfer with exact principal and fee attribution;
2. Jupiter exact-input SOL-to-USDC swap from reviewed state with signer rebinding;
3. classic SPL transfer with ATA creation and later historical re-audit;
4. simulation failure, stale-blockhash expiry, insufficient-funds rejection, and
   same-signature unknown-outcome reconciliation;
5. six real child-process crash/restart checkpoints against Surfpool;
6. native stake create/delegate/deactivate/withdraw lifecycle with exact phase deltas and
   later confirmation re-audits.

Native stake acceptance advances the Surfpool clock across epochs. It therefore runs last
in the complete demo, after every persistent-restart proof. Moving a restarted node behind
already recorded future-slot transactions would make confirmation semantics ambiguous.

## 7. Soaks And Recovery

The canonical real-chain soak is:

```bash
./scripts/surfpool-soak.sh --transactions 1000
```

It uses bounded concurrency, injects exactly one lost send response only after that
signature is observable, reconstructs the runtime, restarts Surfpool against the same
database, and reconciles without resend. Acceptance requires:

- 1,000 confirmed logical actions and 1,000 unique signatures;
- exact payer debit equal to transferred principal plus transaction fees;
- one response-loss recovery and at least one visibility barrier;
- one runtime reconstruction and one Surfpool process restart;
- zero failures, duplicates, budget violations, or unresolved Submitted/Unknown actions;
- peak workers no greater than configured concurrency.

The process-crash matrix runs six independent child processes against the real loopback
Surfpool gateway. Each child persists a checkpoint marker and flushes it to disk; the
parent then sends `SIGKILL`, independently inspects chain and SQLite state, and starts a
second child against the same database. The checkpoints are after intent persistence,
prepared-transaction persistence, simulation, local-signature persistence, send response
loss, and confirmation before local promotion. It proves five confirmed actions, one
unsubmitted expired signature, no duplicate submit attempt or local signature, one event
trace per logical action, and exact principal-plus-fee balance deltas where a transaction
landed.

The 1,000-transaction soak supplies a separate aggregate proof: one injected response
loss, one runtime reconstruction, one persistent Surfpool-process restart, and final
reconciliation without resend.

## 8. Isolation And Secrets

- Full-demo runs allocate unique database, PID, key, session, runtime-env, and log paths.
- A caller may override ports and paths, but all generated keys must stay below the
  project's ignored `.surfpool/keys` boundary, or `.devnet/keys` for the separate devnet
  payer.
- Keys, signed bytes, mutable SQLite files, raw logs, and decompressed snapshots are never
  committed. Full signatures are never committed for loopback runs, where they mean nothing
  outside the surfnet that produced them; the separate devnet record commits them in full
  because they are public record and are the evidence.
- Evidence for this Surfpool harness contains shortened local signatures, public state
  deltas, version/hash provenance, and explicit `public_chain_rpc_reads: 0` and
  `public_network_writes: 0`.
- Raw and sanitized outputs must remain below `evidence/raw` and `evidence`, respectively;
  traversal paths and overwrite attempts are rejected.

## 9. Historical Limitation

The pinned offline snapshot does not contain complete historical mainnet transaction bodies.
Adapter fidelity therefore relies on fresh transactions executed inside Surfpool,
evaluator correctness relies on synthetic known ground truth, and the public report does
not describe the snapshot as full historical-chain validation.

The reviewed Jupiter snapshot is intentionally frozen at slot `433717382`. It is an old,
route-specific 21-account fixture and cannot establish current liquidity, price, or router
behavior. The harness is local and single-host; it does not test remote coordinators,
multi-host stores, public RPC metadata, or mainnet inclusion.

If a chain-facing path cannot be reproduced through Surfpool, it is incomplete. Direct
devnet or mainnet testing is not a development fallback.

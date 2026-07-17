# Surfpool Development Contract

Surfpool is mandatory for every chain-facing development, integration, recovery, and soak
workflow in Account Cooker.

## 1. Pinned Environment

Initial tool contract:

- Surfpool CLI: 1.4.0.
- Expected local binary: surfpool on PATH.
- Verified binary SHA-256 on the planning machine:
  11937d7655bc3a04dd90ae6e6038b9b9db59f2e0d48e1bed35b214fcae3dea24.
- Application RPC: http://127.0.0.1:8899.
- Application WebSocket: ws://127.0.0.1:8900.

The implementation drives the CLI and JSON-RPC. It does not mix Surfpool 1.4.0 with a
different surfpool-sdk crate version.

## 2. Canonical Interactive Network

The planned start script performs:

    mkdir -p .surfpool/keys .surfpool/state .surfpool/logs
    solana-keygen new --silent --no-bip39-passphrase --force \
      --outfile .surfpool/keys/funder.json
    surfpool start \
      --network mainnet \
      --host 127.0.0.1 \
      --port 8899 \
      --ws-port 8900 \
      --no-deploy \
      --no-tui \
      --no-studio \
      --db .surfpool/state/noise.sqlite \
      --surfnet-id noise-dev \
      --airdrop-keypair-path .surfpool/keys/funder.json \
      --airdrop-amount 1000000000000 \
      --max-profiles 2000 \
      --log-path .surfpool/logs

The mainnet network is a read datasource for lazy account hydration. All application
transactions execute locally inside Surfpool.

Use 127.0.0.1 explicitly. Do not depend on global Solana CLI configuration.

## 3. Fail-Closed Network Guard

Before loading a signer, opening the runtime store, or claiming an action:

1. Parse RPC and WebSocket URLs into Surfpool-only validated types.
2. Require an IP-loopback host.
3. Call getVersion and require a surfnet-version field.
4. Call surfnet_getSurfnetInfo successfully.
5. Record Surfpool, Solana-core, genesis/network, and Surfnet identity.
6. Compare the identity to any persisted run being resumed.

The process exits before signer loading for:

- https://api.mainnet-beta.solana.com;
- https://api.devnet.solana.com;
- any non-loopback hostname or IP;
- a local Solana validator that does not answer Surfpool-specific RPC;
- a different Surfnet identity when resuming durable state.

A negative integration test proves each failure.

## 4. Two Test Modes

### Live acceptance mode

- Starts the canonical mainnet-shaped Surfpool fork.
- Lazily fetches current program and account state.
- Uses live public quote/instruction APIs when needed.
- Builds with Surfpool blockhashes.
- Simulates, submits, confirms, and observes locally.
- Captures the pre-state required for deterministic fixtures.

This mode has network reads but no remote Solana writes.

### Deterministic CI mode

Planned shape:

    surfpool start \
      --offline \
      --snapshot fixtures/surfpool/account-cooker-v1.json \
      --host 127.0.0.1 \
      --port 8899 \
      --ws-port 8900 \
      --no-deploy \
      --ci \
      --db :memory: \
      --surfnet-id noise-ci-<run-id>

CI uses:

- reviewed account/program snapshots;
- recorded off-chain quote/instruction responses;
- real transaction building and real program execution inside Surfpool;
- a fresh Surfnet ID and in-memory database per integration group.

Successful swaps are not mocked. Recorded responses supply deterministic planning inputs;
the snapshotted programs and accounts execute the transaction.

## 5. Funding And Token Setup

- SOL comes from Surfpool's startup airdrop to the generated funder.
- Agent wallets receive bounded local funding actions through the cooker.
- Token fixtures use Surfpool token-account cheatcodes or reviewed snapshots.
- USDC uses the canonical mint only inside the local Surfnet.
- Generated keypairs live under .surfpool/keys and are ignored.
- No fixture keypair is committed.

Where clock-sensitive program state is used, align the Surfnet clock forward through the
Surfpool time-travel RPC before execution. Never move it backward across persisted actions.

## 6. Jupiter Workflow

Jupiter is an off-chain planner only:

1. Request a quote from the documented public quote endpoint.
2. Request swap instructions, not a submitted transaction.
3. Apply route, program, mint, amount, slippage, and price-impact policy.
4. Hydrate referenced accounts through Surfpool.
5. Obtain the recent blockhash from Surfpool.
6. Build and sign the versioned transaction locally.
7. Resolve lookup tables through Surfpool.
8. Simulate through Surfpool.
9. Submit and confirm through Surfpool.
10. Assert token balance deltas and route receipts through Surfpool.

Initial live acceptance constrains the route to a known supported DEX to cap account and
snapshot breadth. The selected route and program hashes are recorded in evidence.

No Jupiter or third-party endpoint receives a signed transaction.

## 7. Fixture Capture

After a successful live adapter transaction:

1. export the pre-transaction Surfpool snapshot for accounts/programs touched;
2. normalize and sort account maps;
3. detect conflicting duplicate account definitions;
4. attach metadata;
5. review size, ownership, and secrets;
6. commit the deterministic JSON fixture.

Metadata includes:

- Surfpool and Solana-core versions;
- source slot and block time;
- local source transaction signature;
- adapter and route label;
- program IDs and ProgramData hashes;
- mint IDs and lookup tables;
- capture tool version;
- snapshot content hash.

The mutable Surfpool SQLite database is never committed.

## 8. Required Adapter Scenarios

For each adapter:

- successful plan, simulation, submission, confirmation, and observation;
- insufficient balance;
- missing required account and account-creation path where supported;
- policy-rejected mint/program/destination;
- stale blockhash before submission;
- simulation failure;
- unknown submission response;
- idempotent reconciliation after restart;
- exact pre/post lamport or token assertions;
- local getTransaction logs captured.

No adapter is called complete from instruction construction alone.

## 9. Fault Scenarios

Surfpool scenarios or cheatcodes exercise:

- paused and advanced clock;
- expired blockhash;
- offline/missing account;
- low balance;
- adverse token or pool state;
- process crash before send;
- cooker crash after send and before confirmation persistence;
- Surfpool interruption after send;
- Surfpool restart with the same database and Surfnet ID;
- concurrent workers contending for one wallet;
- kill switch before a local signature;
- daily budget exhaustion.

The postcondition for every unknown-outcome fault is one logical action and at most one
landed transaction.

## 10. Isolation

- Tests do not share mutated Surfnet state.
- Each test group gets a fresh ID and database.
- Interactive evidence runs use a named persistent Surfnet.
- Ports are checked before startup.
- Scripts terminate only processes they started and record their PIDs.
- No broad process-kill command is used.
- Logs and profiles stay with the evidence run until summarized.

## 11. Historical Data Limitation

Surfpool lazy cloning does not guarantee complete historical mainnet transaction bodies.

Therefore:

- adapter tests rely on fresh local transactions;
- evaluator correctness relies on synthetic known ground truth;
- chain-fidelity reports use cooker-generated local receipts;
- any observed-reference fixture states its separate provenance;
- no report describes a lazy fork as full historical-chain validation.

Jito bundle behavior is not part of P0 because Surfpool does not emulate Jito's external
bundle service.

## 12. Evidence Runs

Evidence mode must retain profiles and logs, so it does not use the quiet --ci preset.

Capture:

- Surfpool binary version and hash;
- start command and Surfnet identity;
- snapshot/scenario hashes;
- application revision and config hash;
- model and seeds;
- local signatures;
- transaction profiles and logs;
- pre/post balances;
- application SQLite journal summary;
- restart/fault results;
- evaluator metrics.

All signatures are local Surfpool signatures and are labeled as such.

## 13. Development Rule

If a chain-facing path cannot be reproduced through Surfpool, it is incomplete.

Direct devnet or mainnet testing is not a development fallback. Any later public-network
proof requires a separate explicit decision outside P0.

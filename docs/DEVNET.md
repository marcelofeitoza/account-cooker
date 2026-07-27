# Public Devnet Run And Topology Delta

Status: one bounded sustained run against public Solana devnet is committed, with full
transaction signatures that resolve on a public explorer. The loopback Surfpool results are
unchanged and stay the canonical acceptance evidence. This document exists because a
loopback result cannot, by itself, say anything about public-network topology, and several
of the project's measurements quietly depend on that topology.

This is not a sustained-load proof, and devnet is not mainnet. Section 6 states exactly what
that costs.

## 1. What Was Run

`crates/cooker-solana/tests/devnet_soak.rs`, driven by `scripts/devnet-soak.sh`, executes a
bounded native-SOL workload against `https://api.devnet.solana.com` using the same
components as the loopback soak: the same SQLite store and migrations, the same
`RuntimeEngine` and bounded worker pool, the same `SafetyPolicy`, and the same
`NativeTransferAdapter`. Only the gateway target differs.

The run also reproduces the two durability scenarios from the loopback soak that are
network independent:

- one injected send-response loss, reconciled by signature without resubmission;
- one full runtime-stack restart, where every process-local component is dropped and rebuilt
  from the durable WAL file mid-run.

The Surfpool process restart from the loopback soak has no devnet counterpart. A public
cluster cannot be restarted, so that provenance is absent here rather than faked.

**No on-chain program is deployed by this project.** Account Cooker has no program of its
own; it composes existing Solana programs. Every transaction in this run is a System Program
transfer, so the program id in the evidence is the System Program,
`11111111111111111111111111111111`.

## 2. Reproduce It

Fund exactly one payer, then let it fund anything else by system transfer. The devnet faucet
is rate limited per source address, so airdropping per key fails almost immediately.

```bash
cargo run -p account-cooker -- keygen --output .devnet/keys/payer.json
solana transfer --url https://api.devnet.solana.com --allow-unfunded-recipient \
  "$(solana-keygen pubkey .devnet/keys/payer.json)" 1
./scripts/devnet-soak.sh --transactions 200 --promote
```

`.devnet/` is git ignored, the keypair is created `0600` under a `0700` directory, and the
signer loader refuses any path outside `.surfpool/keys` or `.devnet/keys`.

The script refuses to start unless the endpoint reports the devnet genesis hash
`EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG`, and the gateway proves the same hash again
before a signer is loaded.

Leaving loopback requires naming the cluster in Rust source. `RpcEndpoint::public_cluster`
takes a `PublicCluster` value; the `FromStr` and `TryFrom<Url>` paths that parsed
configuration can reach still accept loopback addresses only. The `cooker` CLI has no
public-network execution path, `scripts/full-demo.sh` never runs this soak, and CI never
runs it either: the test carries `#[ignore]` and CI runs only the default test set.

The gateway paces itself under the endpoint's published request, per-method, and connection
ceilings, and retries transient read failures with bounded exponential backoff.
`sendTransaction` is never retried, so a failed send stays an ambiguous outcome that only
signature reconciliation can settle. Those limiters are active for public-cluster endpoints
only; loopback behavior is byte-for-byte what it was.

The pacing constants were tuned against the real endpoint, and the tuning is itself a result
worth reporting. The endpoint publishes its budgets in `x-ratelimit-*` response headers,
including separate request-rate, per-method, concurrent-connection, and connection-rate
counters. Runs that were nominally inside those ceilings were rejected anyway, because the
counters are a budget for a source address rather than for one process. Successive runs at
130 ms and then 200 ms between requests still drew HTTP 429 responses, so the committed
setting is a small fraction of the reported ceiling.

No rejection lost or duplicated an action. A rejected read was retried; a rejected send
became an ambiguous outcome that reconciliation settled by signature. They did make the run
slow, and they are the reason a bounded run takes tens of minutes rather than the seconds
the same workload takes on loopback.

## 3. Committed Evidence

`evidence/devnet/devnet-soak.json` carries the full run record, including every transaction
signature in submission order with its state, slot, and exact fee. Signatures are committed
verbatim rather than redacted: a devnet signature is public record, and redacting it would
destroy the only thing that makes this run checkable by someone else.

Check any confirmed signature at:

```text
https://explorer.solana.com/tx/<signature>?cluster=devnet
```

Or against the cluster directly:

```bash
curl -s -X POST https://api.devnet.solana.com -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"getTransaction","params":["<signature>",
      {"encoding":"json","maxSupportedTransactionVersion":0}]}'
```

The committed run has no unconfirmed action, but the schema carries them and they are
equally checkable when present: the cluster returns `null` for a signature it never included,
which is exactly what the engine claimed about it.

## 4. Measured Results

The committed run submitted 200 transactions, all 200 confirmed with proven postconditions,
in 622 seconds of wall time at 0.32 confirmations per second, landing in 200 distinct slots
across slots 479,323,050 to 479,324,740 of epoch 1109. Five signatures spread across the run
were spot checked against the cluster afterwards and each matched the recorded slot and fee.

See `evidence/devnet/README.md` for the full numbers, the invariant table, the loopback
comparison, and an account of the earlier attempts that were abandoned rather than
committed.

## 5. Topology Delta

This is the section the evidence exists to support.

### 5.1 What actually differs

| Property | Loopback Surfpool | Public devnet |
|---|---|---|
| Consensus | none; a single local surfnet produces blocks by itself | real leader schedule, votes, and fork choice |
| Inclusion decision | effectively an in-process call that always lands | the current or next slot leader decides, and may drop the transaction |
| Competing traffic | none; every block contains only this run | shared blocks with all other devnet traffic |
| RPC | localhost, plain HTTP, no rate limit, sub-millisecond | shared TLS endpoint with request, per-method, and connection ceilings, all per source address |
| Latency | negligible | real internet round trips plus block time |
| Clock | controllable; the harness time-travels across epochs | not controllable; slot progression is external |
| Blockhash expiry | never reached in practice | a hard 150-slot deadline that abandoned faster-paced attempts did hit |
| Fee market | static base fee only | base fee plus a real priority-fee market under load |
| Account state | frozen mainnet-fork snapshot pinned by hash | live cluster state that moves under the run |
| Restartability | the validator process can be killed and resumed mid-run | not possible |

### 5.2 Which claims depend on that difference

The evaluator's attacks read behavioral features off the action trace. They split cleanly:

**Unaffected by topology.** Amounts, action sequence, destination reuse, route, fee payer,
funding graph, balance rank, and consolidation shape are properties of the transactions
themselves. The same signed bytes produce the same features whichever network executes them.
Every loopback evaluator result that rests only on these features carries over unchanged.

**Affected, in the defender's favor.** Timing features. On loopback the on-chain timestamp
is essentially the scheduler's intended timestamp, so an attacker reading timestamps reads
the scheduler almost directly. On a public network, inclusion delay, leader rotation, and
variable block time insert noise between the intended schedule and the timestamp an analyst
observes. That noise works against the attacker, so the loopback timing numbers are, if
anything, pessimistic for the defender. This is an argument from mechanism, not a
measurement: the evaluator has not been rerun on public-network timestamps.

**Affected, against the defender.** Synchrony features. Inclusion granularity on a public
network is the slot, roughly 400 ms. Two fleet agents the scheduler deliberately separated by
less than a slot can land in the same block and look perfectly synchronized to an observer,
an artifact loopback cannot produce because there is no slot contention. The bounded run
gives one data point: its 200 confirmed transactions landed in 200 distinct slots, so no two
of them collapsed into the same block. That is expected at 0.32 transactions per second and
says nothing about the effect at fleet scale, where the whole point is many agents acting in
the same minute. Characterizing slot collapse needs a run whose submission rate approaches
one transaction per slot, which the shared endpoint's rate budget does not allow.

**An adversary loopback cannot represent at all.** The infrastructure observer. On loopback
there is no third party between the controller and the chain, so the threat model's
infrastructure observer is purely hypothetical. On a public network it is real and concrete:
one RPC provider sees every request from one source address, in order, with timing, before
inclusion. A fleet run this way is trivially linkable at the RPC layer no matter how good its
on-chain behavior is. Nothing in this project defends that channel, and the devnet run makes
the gap concrete rather than theoretical. Any real deployment would need separate endpoints
or network isolation per agent, which is out of scope here and is not claimed.

**A channel loopback cannot show at all.** Action loss. Two distinct mechanisms drop actions
on a public network and neither exists on loopback. The committed run lost none, but earlier
attempts at a higher request rate against the same endpoint lost several, so the mechanisms
are real rather than hypothetical. A transaction can reach the cluster and
never be included before its blockhash expires, and a request can be rejected by the shared
RPC endpoint's rate limiter before the cluster ever sees it. The committed evidence separates
these two causes rather than merging them into one failure count, because they mean different
things: the first is the cluster declining, the second is the operator's own endpoint budget.
Either way the intended action does not happen, so the realized behavior distribution drifts
from the planned one in a way it never does on loopback. The evaluator scores the planned
trace. Scoring realized public-network traces is not done here and is an open gap, stated as
such.

### 5.3 Which loopback numbers carry over

**Carry over, and were re-proven on devnet.** Every durability invariant: no duplicate
logical intent under repeated enqueue, no duplicate signature, exact per-transaction source
debit and destination credit, budget and fee ceilings enforced, ambiguous submissions
reconciled by signature without resubmission, and a full runtime-stack restart recovered
from the WAL file mid-run.

**Do not carry over.** Throughput and latency. The loopback soak is bounded by the engine;
the devnet run is bounded by the shared RPC endpoint's rate budget, which is a property of the
endpoint and the source address, not of the engine. The committed devnet run confirmed every
planned action, but that is a measurement at one pacing setting at one moment, not a property
that transfers: faster-paced attempts against the same endpoint lost transactions, and a
loopback run cannot lose one at all. Nothing that depends on Surfpool-only RPC carries over
either. `surfnet_getSurfnetInfo`, epoch and slot time travel, and local signature listing have
no public-cluster equivalent, so the stake lifecycle acceptance path, which advances Surfpool
across epochs deliberately, stays loopback only.

**Established by neither.** The evaluator's privacy metrics against public-network
timestamps and public-network transaction loss. Both runs measure the engine. Neither
measures whether a fleet looks like human traffic on a live cluster.

## 6. Bounds On This Result

- Devnet is not mainnet. It carries far lower and more erratic load, no meaningful
  priority-fee competition, and a validator set that usually runs ahead of mainnet.
  Contention and fee-market behavior under real economic pressure are untested.
- A bounded run is not a sustained-load proof. The run submits a few hundred transactions
  from one payer over tens of minutes. It says nothing about hours of continuous operation,
  memory or database growth over time, or behavior across a devnet epoch boundary or restart.
- One payer, one process, one RPC endpoint, one region. Nothing here tests a distributed
  controller or multiple endpoints.
- The confirmation rate is a measurement of this endpoint at this moment, not a service
  level. The evidence gate deliberately asserts the engine invariants and only reports the
  confirmation rate, because inclusion is the cluster's decision and gating on it would
  reward tuning the number rather than reporting it.
- The committed run confirmed all 200 planned actions, and that is not a claim about
  reliability. It is one bounded run at one pacing setting from one address. Earlier attempts
  at a higher request rate against the same endpoint had submissions rejected with HTTP 429
  and transactions that expired without inclusion, and were abandoned rather than committed.
  When actions do not confirm, the evidence separates a cluster non-inclusion from an
  endpoint-side rejection so the two are never conflated, because only the first says
  anything about the cluster.
- Destination addresses are derived deterministically from the run identifier and have no
  private key, so the lamports delivered to them are unrecoverable. That is acceptable for
  valueless devnet SOL and would not be acceptable anywhere else.

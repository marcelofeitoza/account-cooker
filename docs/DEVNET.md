# Public Devnet Run And Topology Delta

Status: two runs against public Solana devnet are committed. A 3.32-hour sustained run
(200 rounds, 816 confirmed actions, section 1a) and an earlier 621.9-second bounded run
are
committed, with full transaction signatures that resolved on a public explorer immediately
after the run. The loopback Surfpool results are unchanged and stay the canonical acceptance
evidence. This document exists because a loopback result cannot, by itself, establish
public-network behavior.

This is not a sustained-load proof, and devnet is not mainnet. Section 6 states exactly what
that costs.

## 1. What Was Run

`crates/cooker-solana/tests/devnet_soak.rs`, driven by `scripts/devnet-soak.sh`, executes a
bounded native-SOL workload against `https://api.devnet.solana.com` using the same
components as the loopback soak: the same SQLite store and migrations, the same
`RuntimeEngine` and bounded worker pool, the same `SafetyPolicy`, and the same
`NativeTransferAdapter`. It reuses those application components, while the gateway mode, run
size, pacing, and public-network controls are specific to this run.

The committed record is one payer sending 200 transfers. Every action uses the System Program
and transfers exactly 1,000,000 lamports to a unique deterministic destination. This is not a
multi-payer fleet, sustained-load, behavioral, or anonymity evaluation.

The run also reproduces the two durability scenarios from the loopback soak that are
network independent:

- one injected send-response loss, reconciled by signature without resubmission;
- one in-process execution-component reconstruction. The first `RuntimeEngine` is dropped.
  A new engine is constructed with a reopened store handle, reloaded signer, fresh
  `SafetyPolicy`, `NativeTransferAdapter`, `AdapterContext`, and `SystemClock`, plus the
  original `SolanaGateway` object. The Account Cooker OS process and Tokio runtime remain
  alive. Action state is read from the same SQLite database operating in WAL mode.

The Surfpool process restart from the loopback soak has no devnet counterpart. This harness
does not control public-cluster processes, so that provenance is absent here.

**No on-chain program is deployed by this project.** Account Cooker has no program of its
own; it composes existing Solana programs. Every transaction in this run is a System Program
transfer, so the program id in the evidence is the System Program,
`11111111111111111111111111111111`.

## 1a. The Sustained Run

The 621.9-second run answered "does this work on a public cluster". It could not answer
"does it still behave the same way after hours of real cluster time", because ten minutes
is not long enough for drift, growth or degradation to appear.

This run is the second measurement. One payer, 200 rounds at one round per minute,
**3.32 hours** of wall clock (11,961 s), 816 confirmed actions.

| property | value |
|---|---|
| rounds | 200 |
| wall clock | 11,961 s (3.32 h) |
| confirmed actions | 816 |
| unconfirmed | 0 |
| expired | 0 |
| distinct block leaders observed | 13 |
| epoch | 1110 throughout |
| local database growth | 7,962,488 -> 16,473,808 bytes (+8.5 MB) |
| round execution time | min 13,576 ms, max 51,042 ms |
| first 10 rounds vs last 10 | 18,237 ms avg -> 14,399 ms avg |

**What the duration actually revealed**, none of which the short run could show:

- **No degradation.** The last ten rounds ran *faster* than the first ten (14,399 ms vs
  18,237 ms). Whatever the early cost was, it was warm-up, not accumulation.
- **Storage growth is real and linear.** The local store grew 8.5 MB over 200 rounds,
  about 43 KB per round. That is a capacity-planning fact a ten-minute run cannot surface,
  and it is the strongest argument for measuring duration at all.
- **Cluster variance is wide but not fatal.** The slowest round took 51 s against a 13.6 s
  floor, roughly 3.8x, with zero unconfirmed and zero expired across the whole run.
- **13 distinct leaders**, so the result is not an artifact of one friendly validator.

### Honest bounds on this run specifically

- **It was stopped deliberately at 3.32 h against a 6 h target.** The measurement had
  stabilised and the remaining time was not worth its cost. It is reported as a 3.32-hour
  run, not as the 6-hour run that was planned.
- **Still one payer, one process, one endpoint, one region.** Duration was the variable
  under test; concurrency and topology were not.
- **It stayed inside a single epoch** (1110), so it says nothing about epoch-boundary
  behaviour, which was one of the things a 24-hour run would have covered.
- The devnet faucet refused this address throughout, so the run was budgeted against a
  fixed non-replenishable balance rather than topped up.

Evidence: `evidence/devnet-sustained/` holds the per-round JSONL, the full soak record
with all 816 signatures, a summary, and checksums. Every signature resolves on the public
explorer.

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
public-network execution path, `scripts/full-demo.sh` never runs this test, and CI never
runs it either: the test carries `#[ignore]` and CI runs only the default test set.

The gateway paces itself under the endpoint's published request, per-method, and connection
ceilings, and retries transient read failures with bounded exponential backoff.
`sendTransaction` is never retried, so a failed send stays an ambiguous outcome that only
signature reconciliation can settle. Those limiters are active for public-cluster endpoints
only; loopback requests do not use the public pacing and retry policy.

The pacing constants were tuned against the real endpoint, and the tuning is itself a result
worth reporting. The endpoint publishes its budgets in `x-ratelimit-*` response headers,
including separate request-rate, per-method, concurrent-connection, and connection-rate
counters. Runs that were nominally inside those ceilings were rejected anyway, because the
counters are a budget for a source address rather than for one process. Successive runs at
130 ms and then 200 ms between requests still drew HTTP 429 responses, so the committed
setting is a small fraction of the reported ceiling.

In the abandoned attempts, rejected reads were retried and rejected sends became ambiguous
outcomes for signature reconciliation rather than automatic resubmission. The committed run
recorded no rejected send. Its 621.9-second duration reflects the conservative pacing selected
after those attempts plus public RPC and inclusion latency.

## 3. Committed Evidence

`evidence/devnet/devnet-soak.json` carries one record per planned action in ascending
`sequence` order, not submission or confirmation order. Each record carries its terminal
state, signature when one was persisted, slot when confirmed, exact fee, and destination.
Signatures are committed verbatim rather than redacted: a devnet signature is public record,
and redacting it would remove the main independently checkable identifier.

The committed schema-2 file is a post-run truth correction of the original schema-1 record,
not a second devnet run. Its `evidence_revision` object preserves the original checksum and
states that no transaction record or measurement changed. Only the reconstruction labels,
scenario wording, and explicit record-order metadata changed.

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
in 621.9 seconds of wall time at 0.323 confirmations per submission-batch second. Confirmed
transactions landed in 200 distinct slots from 479,323,073 through 479,324,703 in epoch 1109;
the before and after observations span absolute slots 479,323,050 through 479,324,740. Five
records selected across the planned sequence were spot checked against the cluster immediately
after the run, and each matched the recorded slot and fee.

See `evidence/devnet/README.md` for the full numbers, the invariant table, the loopback
comparison, and an account of the earlier attempts that were abandoned rather than
committed.

## 5. Topology Delta

This is the section the evidence exists to support.

### 5.1 What actually differs

| Property | Loopback Surfpool | Public devnet |
|---|---|---|
| Consensus | no validator consensus measured; one isolated Surfpool process | public devnet consensus topology, not independently evaluated by this run |
| Inclusion decision | local RPC to one isolated Surfpool process; all canonical soak actions landed | the public cluster can include or drop a submitted transaction |
| Competing traffic | isolated process with no configured external transaction source | shared public cluster |
| RPC | loopback HTTP with no public-provider quota in this harness | shared TLS endpoint with request, per-method, and connection ceilings, all per source address |
| Observed end-to-end rate | 27.091 transactions/s in the 1,000-transaction canonical soak | 0.323 confirmations/s during submission batches in this 200-transaction run |
| Clock | controllable; the harness time-travels across epochs | not controllable; slot progression is external |
| Blockhash expiry | not observed in the committed canonical soak | signature absence through expiry occurred in an abandoned pilot |
| Fee market | canonical transfers paid the local base fee | committed transfers paid 5,000 lamports each and did not exercise a priority fee |
| Account state | frozen mainnet-fork snapshot pinned by hash | live cluster state that moves under the run |
| Harness restart control | the local Surfpool process can be killed and resumed mid-run | the harness cannot restart the public cluster |

### 5.2 Which claims depend on that difference

The evaluator's attacks read behavioral features off the action trace. They split cleanly:

**Not reevaluated by topology.** The evaluator scores planned synthetic action traces. Network
execution topology is not one of its inputs, so the committed evaluator values do not change
because this separate devnet run exists. This does not prove that signed transactions, realized
timing, action loss, or public-network observations are equivalent across networks.

**Timing direction is unknown.** Public inclusion delay, leader rotation, and variable block
time can perturb the relationship between intended schedule and observed timestamp. That may
help or hurt linkage depending on the attacker and feature. The evaluator was not rerun on
public-network timestamps, so this project reports no direction or privacy gain from that
perturbation.

**Synchrony was not tested at fleet scale.** Public inclusion granularity and contention can
place multiple submitted actions in one slot. The bounded run gives one data point: its 200
confirmed transactions landed in 200 distinct slots. At 0.323 confirmations per second from
one payer, that does not characterize multi-agent synchrony or show how either network behaves
near one transaction per slot.

**An adversary loopback cannot represent at all.** The infrastructure observer. On loopback
there is no third party between the controller and the chain, so the threat model's
infrastructure observer is purely hypothetical. On a public network it is concrete: one RPC
provider can observe requests from one source address and their arrival timing before
inclusion. Nothing in this project defends RPC or IP correlation, and this single-endpoint run
does not evaluate it. Separate endpoints or network isolation may change that exposure, but
they are out of scope and untested here.

**Public execution can leave planned actions unrealized.** The committed run lost none.
Operator-observed abandoned attempts produced explicit HTTP 429 send rejections and signatures
that remained absent through blockhash expiry. An explicit edge rejection proves the endpoint
did not accept that request. Signature absence through expiry does not, by itself, prove
whether the endpoint or cluster received the send. The evidence schema keeps these outcomes
separate. Scoring realized public-network traces is not done here; the evaluator scores planned
synthetic traces. The committed loopback soak had no unrealized action, while fault tests cover
other local failure paths.

### 5.3 Which loopback numbers carry over

**Re-proven on devnet.** The run established no duplicate logical intent under repeated
enqueue, no duplicate signature, exact per-transaction source debit and destination credit,
budget and fee ceilings, and ambiguous-submission reconciliation by signature without
resubmission. It also established the in-process component reconstruction described in
section 1. It did not restart the Account Cooker process. The same `SolanaGateway` object was
reused; HTTP and TLS connection reuse was not instrumented.

**Do not carry over.** Throughput and latency. The runs differ in transaction count, client
pacing, gateway mode, and network, and neither experiment isolates a single bottleneck. Their
observed rates are therefore not comparable engine or endpoint capacities. Faster abandoned
devnet attempts encountered HTTP 429 responses and signature absence through expiry. Nothing
that depends on Surfpool-only RPC carries over either. `surfnet_getSurfnetInfo`, epoch and slot
time travel, and local signature listing have no public-cluster equivalent, so the stake
lifecycle acceptance path, which advances Surfpool across epochs deliberately, stays
loopback only.

**Established by neither.** The evaluator's privacy metrics against public-network
timestamps and public-network transaction loss. Both runs measure the engine. Neither
measures whether a fleet looks like human traffic on a live cluster.

## 6. Bounds On This Result

- Devnet is not mainnet. Its topology and economics differ, and this run did not measure
  relative load, validator behavior, contention under economic demand, or a priority-fee
  market.
- A bounded run is not a sustained-load proof. The run submits 200 transactions from one
  payer over 621.9 seconds, about 10.4 minutes. It says nothing about hours of continuous
  operation, memory or database growth over time, or behavior across a devnet epoch boundary
  or Account Cooker OS-process restart.
- One payer, one process, one RPC endpoint, one region. Nothing here tests a distributed
  controller or multiple endpoints.
- The confirmation rate is an end-to-end measurement under this run's committed client
  pacing, endpoint, and cluster conditions, not a service level or isolated endpoint
  capacity. The evidence gate reports it but does not assert a minimum.
- The committed run confirmed all 200 planned actions, and that is not a claim about
  reliability. It is one bounded run at one pacing setting from one address. Earlier attempts
  at a higher request rate against the same endpoint had submissions rejected with HTTP 429
  and transactions that expired without inclusion, and were abandoned rather than committed.
  When actions do not confirm, the evidence separates explicit endpoint-side rejection from
  signature absence through expiry. The latter does not prove where a send was lost.
- Destination addresses are derived deterministically from the run identifier and have no
  private key, so the lamports delivered to them are unrecoverable. That is acceptable for
  valueless devnet SOL and would not be acceptable anywhere else.

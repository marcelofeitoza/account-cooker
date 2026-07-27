# Bounded Public Devnet Record

One bounded native-transfer run of the engine against public Solana devnet, produced by
`scripts/devnet-soak.sh --promote` on 2026-07-27. This directory is not part of the
canonical `evidence/final` pack, which by definition contains no public-network transaction.

Read [the devnet run and topology delta](../../docs/DEVNET.md) before comparing these numbers
to the loopback ones. Several of them do not transfer.

## Contents

- `devnet-soak.json`: the full run record, including every planned action in ascending
  `sequence` order with its terminal state, signature, slot, destination, and exact fee.
- `checksums.txt`: SHA-256 of that file.

Schema 2 is a post-run truth correction, not a second network execution. The
`evidence_revision` object records the original schema-1 checksum and states that transaction
records and measurements were unchanged. The correction replaces the misleading restart label
with the actual in-process reconstruction semantics and declares the existing planned-sequence
ordering. Git history retains the original schema-1 artifact.

## Network

| Field | Value |
|---|---|
| Cluster | `solana-devnet` |
| RPC endpoint | `https://api.devnet.solana.com/` |
| Genesis hash | `EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG` |
| Validator version reported | `4.2.0-beta.1` |
| Feature set | `4119855713` |
| Program id exercised | `11111111111111111111111111111111` (System Program) |
| Payer | `EGwTkJYgva6D64HaM286Dkv3agzsyNbHxNGWtRSA6iGk` |

This project deploys no on-chain program of its own. Every transaction here is a System
Program transfer signed locally by the payer above.

The record is a 621.9-second run from that single payer. All 200 actions have the same
System Program transfer type and 1,000,000-lamport amount, with one unique deterministic
destination per action. It is not a sustained-load, multi-payer fleet, behavioral, or
anonymity evaluation.

## Verify It Yourself

Every signature in `devnet-soak.json` is committed verbatim and is public record.

A confirmed one, which resolves with slot, fee, and no error:

```text
4nTg2zbZ1kTmw4jL7fwmKFjivQnJMzrNdf6uXfbRqb5xCbsSreWLiqFofrXKQrnc9JtM1PV5ky5ZfMUY8RAC8vCm
https://explorer.solana.com/tx/4nTg2zbZ1kTmw4jL7fwmKFjivQnJMzrNdf6uXfbRqb5xCbsSreWLiqFofrXKQrnc9JtM1PV5ky5ZfMUY8RAC8vCm?cluster=devnet
```

Check either against the cluster directly:

```bash
curl -s -X POST https://api.devnet.solana.com -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"getTransaction","params":["<signature>",
      {"encoding":"json","maxSupportedTransactionVersion":0}]}'
```

Five records selected across the planned sequence were spot checked against the cluster
immediately after the run, and each returned the same slot and the same 5,000 lamport fee
recorded here, with no execution error. Devnet retains recent history only, so older
signatures may age out of this endpoint even though they were included; the recorded slot
is the durable reference.

## Measured

| Measurement | Value |
|---|---|
| Planned actions | 200 |
| Confirmed with proven postconditions | 200 |
| Unconfirmed | 0 |
| Inclusion rate | 100.0 percent |
| Worker concurrency | 8 (peak 8) |
| Wall time, whole run | 621.9 s |
| Wall time, submission batches only | 618.5 s |
| Confirmations per second, submission batches | 0.323 |
| Distinct slots holding a confirmed transaction | 200 |
| Absolute slot observations before/after | 479323050 to 479324740 |
| Confirmed transaction slot range | 479323073 to 479324703 |
| Epoch | 1109 to 1109 |
| Fees paid | 1000000 lamports |
| Payer debit | 201000000 lamports |
| Payer balance equation closes | true |
| Durable journal events | 2003 |
| Database size | 2764800 bytes |

### Why each unconfirmed action did not confirm

Every planned action confirmed in this run, so there is nothing to attribute. The classification still exists and the evidence gate refuses a run that reports an unconfirmed action without a cause, because a non-inclusion with no stated reason is not a result.

Two causes are defined. `not_included_before_blockhash_expiry` means the persisted signature
remained absent after the blockhash validity window; that result alone does not prove whether
the endpoint or cluster received the send. `rpc_edge_rate_limited_send` means the shared
public endpoint returned HTTP 429 for the submission request, so that request was rejected at
the endpoint edge.

### Engine invariants, all asserted

| Invariant | Result |
|---|---|
| Duplicate logical intents | 0 |
| Duplicate enqueue attempts rejected idempotently | 200 of 200 |
| Duplicate signatures | 0 |
| Budget violations | 0 |
| Actions left unresolved in Submitted | 0 |
| Actions left unresolved in Unknown | 0 |
| Injected send-response losses | 1 |
| Ambiguous outcomes settled without resubmission | 1 |
| Of those, settled as confirmed | 1 |
| Reconciliation passes needed | 1 |
| In-process execution-component reconstructions from the same SQLite database | 1 |
| Account Cooker OS-process restarts | 0 |
| `SolanaGateway` object reconstructions | 0; the original object was reused |
| HTTP or TLS connection reuse | not instrumented |
| Exact source debit proven | 200 of 200 confirmed |
| Exact destination credit proven | 200 of 200 confirmed |

## Side By Side With The Loopback Soak

The loopback record is `../final/surfpool-soak.json` and is unchanged.

| | Loopback Surfpool | Public devnet | Comparable? |
|---|---|---|---|
| Transactions | 1000 | 200 | sizes differ on purpose |
| Confirmed | 1000 | 200 | yes |
| Inclusion rate | 100 percent | 100.0 percent | yes |
| Wall time | 36.9 s | 621.9 s | no; dimensions, pacing, gateway, and network differ |
| Transactions per second | 27.091 | 0.323 | no; neither run isolates an engine or endpoint capacity |
| Duplicate signatures | 0 | 0 | yes |
| Duplicate logical intents | 0 | 0 | yes |
| Budget violations | 0 | 0 | yes |
| Unresolved after reconciliation | 0 | 0 | yes |
| Response loss reconciled without resend | 1 | 1 | yes |
| Harness-managed Surfpool process restart mid-run | 1 | not under harness control | no |

The rows marked comparable are the engine's asserted invariants and had the same result in
both records. Throughput should not be read as an engine or endpoint characteristic: the
runs use different dimensions, client pacing, gateway modes, and networks, and neither
experiment isolates the bottleneck.

## What It Took To Get Here

This was not the first attempt, and saying so matters more than the clean number above.

Two small pilots of 12 and 16 transactions ran first, at a faster request pacing. The
16-transaction pilot confirmed 15 and recorded 1 as expired: its persisted signature remained
absent through blockhash expiry. That does not establish where the send was lost. Three
subsequent attempts at the full 200 transactions, still at the faster
pacing, were abandoned mid-run once the shared public endpoint began returning HTTP 429 for
submissions at request rates that were nominally inside its own published ceilings. Under
those conditions submissions were rejected before reaching the cluster, and the engine
correctly turned each one into an ambiguous outcome awaiting reconciliation rather than
resubmitting anything.

None of those runs are committed and none of their numbers are claimed here. The committed
run is the one whose pacing settings match the committed source. Backing off reduced the
observed edge rejections; the abandoned attempts do not isolate an endpoint or engine
capacity.

Read the 100 percent inclusion rate accordingly. It is one bounded run, at one pacing, from
one address, at one moment. It is not a reliability claim.

## Not In This Record

No keypair, no signed transaction bytes, no database, and no raw RPC log. Those stay in the
git-ignored `.devnet` and `evidence/raw` directories. Signatures are the exception and are
committed in full on purpose: they are already public and are the direct identifiers needed
to query and verify each recorded transaction.

# Bounded Public Devnet Record

One bounded sustained run of the engine against public Solana devnet, produced by
`scripts/devnet-soak.sh --promote` on 2026-07-27. This directory is not part of the
canonical `evidence/final` pack, which by definition contains no public-network transaction.

Read [the devnet run and topology delta](../../docs/DEVNET.md) before comparing these numbers
to the loopback ones. Several of them do not transfer.

## Contents

- `devnet-soak.json`: the full run record, including every planned action with its terminal
  state, signature, slot, and exact fee.
- `checksums.txt`: SHA-256 of that file.

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

Five signatures spread across the run were spot checked against the cluster immediately
after it finished, and each returned the same slot and the same 5,000 lamport fee recorded
here, with no execution error. Devnet retains recent history only, so older signatures may
age out of this endpoint even though they were included; the recorded slot is the durable
reference.

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
| Slot span of the run | 479323050 to 479324740 |
| Epoch | 1109 to 1109 |
| Fees paid | 1000000 lamports |
| Payer debit | 201000000 lamports |
| Payer balance equation closes | true |
| Durable journal events | 2003 |
| Database size | 2764800 bytes |

### Why each unconfirmed action did not confirm

Every planned action confirmed in this run, so there is nothing to attribute. The classification still exists and the evidence gate refuses a run that reports an unconfirmed action without a cause, because a non-inclusion with no stated reason is not a result.

Two causes are defined. `not_included_before_blockhash_expiry` means the cluster received the transaction and did not include it inside the 150-slot validity window. `rpc_edge_rate_limited_send` means the shared public endpoint returned HTTP 429 for the submission, so the cluster never saw it, which is a property of the free endpoint and the address calling it rather than of the cluster or the engine.

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
| Runtime stack restarts recovered from the WAL file | 1 |
| Exact source debit proven | 200 of 200 confirmed |
| Exact destination credit proven | 200 of 200 confirmed |

## Side By Side With The Loopback Soak

The loopback record is `../final/surfpool-soak.json` and is unchanged.

| | Loopback Surfpool | Public devnet | Comparable? |
|---|---|---|---|
| Transactions | 1000 | 200 | sizes differ on purpose |
| Confirmed | 1000 | 200 | yes |
| Inclusion rate | 100 percent | 100.0 percent | yes |
| Wall time | 36.9 s | 621.9 s | no, different bottlenecks |
| Transactions per second | 27.091 | 0.323 | no; loopback is engine bound, devnet is RPC-quota bound |
| Duplicate signatures | 0 | 0 | yes |
| Duplicate logical intents | 0 | 0 | yes |
| Budget violations | 0 | 0 | yes |
| Unresolved after reconciliation | 0 | 0 | yes |
| Response loss reconciled without resend | 1 | 1 | yes |
| Validator process restart mid-run | 1 | not possible | no |

The rows marked comparable are the engine's own invariants and hold identically on both
networks. The rows marked not comparable are properties of the network and its RPC provider.
Throughput in particular should not be read as an engine characteristic: the devnet figure is
what one address is allowed to ask a free shared endpoint for, and it would change with a
different endpoint without a single line of engine code changing.

## What It Took To Get Here

This was not the first attempt, and saying so matters more than the clean number above.

Two small pilots of 12 and 16 transactions ran first, at a faster request pacing. The
16-transaction pilot confirmed 15 and recorded 1 as expired: that transaction was submitted,
never included, and the cluster returned `null` for its signature, exactly as the engine had
classified it. Three subsequent attempts at the full 200 transactions, still at the faster
pacing, were abandoned mid-run once the shared public endpoint began returning HTTP 429 for
submissions at request rates that were nominally inside its own published ceilings. Under
those conditions submissions were rejected before reaching the cluster, and the engine
correctly turned each one into an ambiguous outcome awaiting reconciliation rather than
resubmitting anything.

None of those runs are committed and none of their numbers are claimed here. The committed
run is the one whose pacing settings match the committed source, which is roughly a third of
the endpoint's published request rate. The reason for backing off was that a run dominated by
endpoint rejections measures the endpoint, not the engine.

Read the 100 percent inclusion rate accordingly. It is one bounded run, at one pacing, from
one address, at one moment. It is not a reliability claim.

## Not In This Record

No keypair, no signed transaction bytes, no database, and no raw RPC log. Those stay in the
git-ignored `.devnet` and `evidence/raw` directories. Signatures are the exception and are
committed in full on purpose: they are already public, and they are the only part of this
record a third party can independently check.

# Classic SPL Live Fixture

This fixture defines the deterministic assertions for the classic SPL Token live acceptance
scenario. The mint and associated token accounts are created at runtime by locally signed
transactions submitted only to the pinned loopback Surfpool RPC. No keypair or mutable Surfnet
database is committed.

Run the complete scenario with:

```sh
scripts/surfpool-live-spl.sh
```

The test creates an initialized classic SPL mint, creates and funds the payer's associated token
account, then invokes the production `SplTransferAdapter` while the destination ATA is absent. It
proves:

- mint program ownership, initialization, decimals, and raw supply;
- source and destination token-account mint and wallet ownership;
- exact raw source and destination balance deltas;
- confirmed transaction metadata for both token accounts;
- the exact transaction fee and exact destination ATA rent charged to the payer; and
- the adapter's complete postcondition receipt.

Addresses and signatures in evidence are public local-ledger identifiers. Signature samples are
truncated, and no secret key material is written to evidence.

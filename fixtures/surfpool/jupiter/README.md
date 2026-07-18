# Jupiter Planner Fixtures

The quote and instruction JSON files are unsigned HTTP response captures for deterministic serde
and policy tests. The compressed account snapshot freezes the reviewed route's public on-chain
state so the same instructions can also be executed repeatedly on an offline local Surfpool.

- Captured: 2026-07-17 from the official `https://lite-api.jup.ag/swap/v1` endpoint.
- Pair: exact-in 0.1 SOL to USDC, 50 bps slippage, direct `Raydium CLMM` route.
- Quote controls: `onlyDirectRoutes=true`, `restrictIntermediateTokens=true`,
  `asLegacyTransaction=false`, `maxAccounts=64`, `instructionVersion=V2`.
- Instruction controls: `useSharedAccounts=false`, `prioritizationFeeLamports=0`,
  `dynamicComputeUnitLimit=false`, `dynamicSlippage=false`,
  `skipUserAccountsRpcCalls=true`.
- Route program: Raydium CLMM `CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK`.
- Reviewed AMM account: `CYbD9RaToYMtWKA7QZyoLahnHdWq553Vm62Lh6qWtuxq`.
- Outer program: Jupiter v6 `JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4`.
- Lookup table: `BDqppwFYeMpUicN9xbfoM7FgRnHVW1uTUtrGA7uG2vQg`.

Recorded pairs:

| Stem | Quote context | Purpose |
| --- | ---: | --- |
| `raydium-clmm-sol-usdc` | 433407240 | Original V2 serde and policy fixture |
| `raydium-clmm-sol-usdc-cyb-433411234` | 433411234 | Fresh CYb privilege manifest, including tick array `F8eHC...` |

SHA-256:

```text
93861c78dc91ee22e5ee44c8712f2fed970c8943209f64b40c9cfbd2427a9ce6  raydium-clmm-sol-usdc.quote.json
d2dd8ce993aeace25f48da84c24d590f76472f0f43ea4ce1f91988c3ceabea22  raydium-clmm-sol-usdc.instructions.json
d05c54f58da2b6d1d8cfe84fb3d023434890e467c567c49d974079a9b96a00d2  raydium-clmm-sol-usdc-cyb-433411234.quote.json
d0b56b02b7285ed44a6c00951c425388c1cc293541bf1c6d385b4fa1f62f6b9d  raydium-clmm-sol-usdc-cyb-433411234.instructions.json
b2c428d69c90388c3dcdfa91f1789c82a6584544885eb6272b55cbb7094aeaab  raydium-clmm-sol-usdc-433717382.snapshot.json
f1efd977ec801baec8856e594d215965414440a8a0158b7fcc607b1bb196f2b1  raydium-clmm-sol-usdc-433717382.snapshot.json.gz
```

The snapshot was exported from Surfpool 1.4.0 at the pre-transaction state for slot 433717382. It
contains 21 public accounts, including the reviewed lookup table and Jupiter/Raydium upgradeable
program-data accounts. The fixture user's account was removed before compression. No keypair,
seed, credential, transaction database, or private RPC material is present.

At execution time, the harness replaces exactly three reviewed identities in instruction account
metadata: the fixture signer and that signer's WSOL and USDC associated token accounts. They are
rebound to a newly generated local signer and its derived accounts. A non-network invariant test
reverses those replacements and proves every other field remains byte-for-byte equivalent after
serialization. Full signatures and local databases stay under the ignored `.surfpool/` tree;
committed acceptance evidence carries shortened signatures only.

During capture, identical 0.1 SOL request shapes also selected the Raydium CLMM pools
`8sLbNZoA1cfnvMJLPfp98ZLAnFSYCFApfJKMbiXNLwxj` and
`5s7njN2X6k3trkibTKX6LJFu4PnybYhCuADP9LD2fhuP` through another backend path. Their exact raw
quote/instruction pairs could not be retained before rate limiting, so they are intentionally not
approved. A live run selecting either pool must fail closed until its full account, writable, and
lookup-table manifest is captured and reviewed.

The snapshot is a deterministic input, not acceptance evidence by itself. The chain acceptance
gate still performs local key generation, instruction rebinding, simulation, submission,
confirmation, exact input/output balance assertions, and historical re-audit on Surfpool.

Official response-shape references:

- <https://developers.jup.ag/docs/api-reference/swap/v1/quote>
- <https://developers.jup.ag/docs/api-reference/swap/v1/swap-instructions>
- <https://developers.jup.ag/docs/api-reference/swap/v1/program-id-to-label>

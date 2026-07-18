# Surfpool Fixtures

Deterministic public adapter fixtures live in this directory. Runtime state does not.

The mutable `.surfpool/state/*.sqlite` database is intentionally ignored. A committed
snapshot comes from `surfnet_exportSnapshot` with `preTransaction` scope and
must be accompanied by reviewed metadata containing:

- Surfpool version and binary hash;
- Solana core version, source slot, and block time;
- local Surfpool transaction signature;
- adapter and route label;
- touched program IDs, ProgramData hashes, mints, and lookup tables;
- normalized snapshot SHA-256.

Snapshots may contain public account state only. Never place a keypair, seed, RPC
credential, raw application database, or mutable Surfpool database in this directory.

The committed Jupiter/Raydium archive has a separate manifest, compressed and
decompressed SHA-256 pins, and no fixture signer. Classic SPL creates every mutable token
account locally. The native staking spike records why the complete Stake-program lifecycle
is the reproducible stateful adapter rather than claiming an unavailable external operator
service.

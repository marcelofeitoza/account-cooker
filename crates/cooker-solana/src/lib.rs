//! Network identity guard, JSON-RPC gateway, local signing, and action adapters.

#![forbid(unsafe_code)]

mod adapters;
mod error;
mod fleet;
mod jupiter;
mod network;
mod rpc;
mod signer;
mod stake;
mod transaction;

pub use adapters::{NativeTransferAdapter, SplTransferAdapter};
pub use error::{RpcError, RpcFailureClass};
pub use fleet::{FleetEntry, FleetManifest};
pub use jupiter::{
    JupiterAccountMeta, JupiterAdapter, JupiterApiClient, JupiterInstruction, JupiterPlatformFee,
    JupiterPolicy, JupiterQuote, JupiterRouteStep, JupiterSwapInfo, JupiterSwapInstructions,
};
pub use network::{PublicCluster, RpcEndpoint, RpcEndpointClass};
pub use rpc::{
    AccountInfo, ClusterIdentity, EXPECTED_SURFPOOL_VERSION, EpochInfo, GatewayIdentity,
    LatestBlockhash, LocalSignatureRecord, RpcSimulation, SignatureStatus, SolanaGateway,
    SurfpoolIdentity, TokenBalanceRecord, TransactionRecord, VoteAccount,
};
pub use signer::LocalKeypair;
pub use stake::NativeStakeAdapter;
pub use transaction::{
    SignedWireTransaction, build_signed_transaction, build_signed_transaction_with_signers,
    build_signed_v0_transaction,
};

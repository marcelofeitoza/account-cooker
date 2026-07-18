//! RPC failure categories that preserve submission ambiguity.

use cooker_core::CookerError;
use thiserror::Error;

const SEND_TRANSACTION: &str = "sendTransaction";

/// Operational meaning of a Surfpool RPC failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RpcFailureClass {
    /// The verified endpoint is not Surfpool or is otherwise unsafe.
    Identity,
    /// Retrying the same request cannot make it valid.
    Deterministic,
    /// The request failed before submission and may be retried with backoff.
    Transient,
    /// A signed transaction may have landed and must be reconciled by signature.
    UnknownOutcome,
}

#[derive(Debug, Error)]
enum RpcErrorKind {
    #[error("unsafe Surfpool identity: {0}")]
    Identity(String),
    #[error("invalid request input: {0}")]
    InvalidInput(String),
    #[error("transport failure: {detail}")]
    Transport {
        detail: String,
        class: RpcFailureClass,
    },
    #[error("HTTP {status}: {detail}")]
    Http {
        status: u16,
        detail: String,
        class: RpcFailureClass,
    },
    #[error("JSON-RPC error {code}: {message}")]
    JsonRpc {
        code: i64,
        message: String,
        class: RpcFailureClass,
    },
    #[error("invalid response: {detail}")]
    InvalidResponse {
        detail: String,
        class: RpcFailureClass,
    },
}

/// Classified error returned by the Surfpool JSON-RPC transport.
#[derive(Debug, Error)]
#[error("Surfpool RPC {method} failed: {kind}")]
pub struct RpcError {
    method: &'static str,
    kind: RpcErrorKind,
}

impl RpcError {
    pub(crate) fn identity(method: &'static str, detail: impl Into<String>) -> Self {
        Self {
            method,
            kind: RpcErrorKind::Identity(detail.into()),
        }
    }

    pub(crate) fn invalid_input(method: &'static str, detail: impl Into<String>) -> Self {
        Self {
            method,
            kind: RpcErrorKind::InvalidInput(detail.into()),
        }
    }

    pub(crate) fn transport(method: &'static str, error: &reqwest::Error) -> Self {
        let class = if method == SEND_TRANSACTION && !error.is_connect() {
            RpcFailureClass::UnknownOutcome
        } else {
            RpcFailureClass::Transient
        };
        Self {
            method,
            kind: RpcErrorKind::Transport {
                detail: sanitize(error.to_string()),
                class,
            },
        }
    }

    pub(crate) fn http(method: &'static str, status: u16, detail: impl Into<String>) -> Self {
        let class = if is_identity_method(method) {
            RpcFailureClass::Identity
        } else if method == SEND_TRANSACTION {
            RpcFailureClass::UnknownOutcome
        } else if status == 429 || status >= 500 {
            RpcFailureClass::Transient
        } else {
            RpcFailureClass::Deterministic
        };
        Self {
            method,
            kind: RpcErrorKind::Http {
                status,
                detail: sanitize(detail.into()),
                class,
            },
        }
    }

    pub(crate) fn json_rpc(method: &'static str, code: i64, message: impl Into<String>) -> Self {
        let class = if is_identity_method(method) {
            RpcFailureClass::Identity
        } else if matches!(code, -32602..=-32600 | -32002) {
            RpcFailureClass::Deterministic
        } else {
            RpcFailureClass::Transient
        };
        Self {
            method,
            kind: RpcErrorKind::JsonRpc {
                code,
                message: sanitize(message.into()),
                class,
            },
        }
    }

    pub(crate) fn invalid_response(method: &'static str, detail: impl Into<String>) -> Self {
        let class = if is_identity_method(method) {
            RpcFailureClass::Identity
        } else if method == SEND_TRANSACTION {
            RpcFailureClass::UnknownOutcome
        } else {
            RpcFailureClass::Deterministic
        };
        Self {
            method,
            kind: RpcErrorKind::InvalidResponse {
                detail: sanitize(detail.into()),
                class,
            },
        }
    }

    /// Return the RPC method whose outcome failed.
    #[must_use]
    pub const fn method(&self) -> &'static str {
        self.method
    }

    /// Return the retry and reconciliation class for this failure.
    #[must_use]
    pub const fn class(&self) -> RpcFailureClass {
        match &self.kind {
            RpcErrorKind::Identity(_) => RpcFailureClass::Identity,
            RpcErrorKind::InvalidInput(_) => RpcFailureClass::Deterministic,
            RpcErrorKind::Transport { class, .. }
            | RpcErrorKind::Http { class, .. }
            | RpcErrorKind::JsonRpc { class, .. }
            | RpcErrorKind::InvalidResponse { class, .. } => *class,
        }
    }

    /// Convert into the runtime-wide error type without losing ambiguous sends.
    #[must_use]
    pub fn into_cooker(self) -> CookerError {
        let detail = self.to_string();
        match self.class() {
            RpcFailureClass::Identity => CookerError::InvalidConfig(detail),
            RpcFailureClass::UnknownOutcome => CookerError::UnknownOutcome(detail),
            RpcFailureClass::Deterministic | RpcFailureClass::Transient => {
                CookerError::Chain(detail)
            }
        }
    }
}

impl From<RpcError> for CookerError {
    fn from(error: RpcError) -> Self {
        error.into_cooker()
    }
}

fn is_identity_method(method: &str) -> bool {
    matches!(method, "getVersion" | "surfnet_getSurfnetInfo")
}

fn sanitize(detail: impl AsRef<str>) -> String {
    detail.as_ref().chars().take(512).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_rpc_rejection_is_not_an_ambiguous_send() {
        let error = RpcError::json_rpc(SEND_TRANSACTION, -32002, "simulation failed");
        assert_eq!(error.class(), RpcFailureClass::Deterministic);
    }

    #[test]
    fn malformed_send_response_is_ambiguous() {
        let error = RpcError::invalid_response(SEND_TRANSACTION, "connection ended after write");
        assert_eq!(error.class(), RpcFailureClass::UnknownOutcome);
        assert!(matches!(
            error.into_cooker(),
            CookerError::UnknownOutcome(_)
        ));
    }
}

//! Loopback-only Surfpool endpoint validation.

use std::{fmt, net::IpAddr, str::FromStr};

use url::{Host, Url};

use crate::RpcError;

/// Validated HTTP endpoint that can only address an IP loopback interface.
#[derive(Clone, Eq, Hash, PartialEq)]
pub struct SurfpoolRpcUrl(Url);

impl SurfpoolRpcUrl {
    /// Validate an already parsed URL.
    ///
    /// # Errors
    ///
    /// Returns [`RpcError`] unless the URL is plain HTTP, has an explicit loopback IP and port,
    /// and contains no credentials, path, query, or fragment.
    pub fn new(url: Url) -> Result<Self, RpcError> {
        if url.scheme() != "http" {
            return Err(RpcError::identity(
                "connect",
                "Surfpool RPC must use plain HTTP on loopback",
            ));
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(RpcError::identity(
                "connect",
                "Surfpool RPC URL must not contain credentials",
            ));
        }
        if url.query().is_some() || url.fragment().is_some() || !matches!(url.path(), "" | "/") {
            return Err(RpcError::identity(
                "connect",
                "Surfpool RPC URL must not contain a path, query, or fragment",
            ));
        }
        let ip = match url.host() {
            Some(Host::Ipv4(ip)) => IpAddr::V4(ip),
            Some(Host::Ipv6(ip)) => IpAddr::V6(ip),
            Some(Host::Domain(_)) => {
                return Err(RpcError::identity(
                    "connect",
                    "Surfpool RPC host must be an explicit loopback IP",
                ));
            }
            None => {
                return Err(RpcError::identity(
                    "connect",
                    "Surfpool RPC URL has no host",
                ));
            }
        };
        if !ip.is_loopback() {
            return Err(RpcError::identity(
                "connect",
                "Surfpool RPC host is not loopback",
            ));
        }
        url.port().ok_or_else(|| {
            RpcError::identity("connect", "Surfpool RPC URL must include an explicit port")
        })?;
        Ok(Self(url))
    }

    /// Borrow the validated URL.
    #[must_use]
    pub const fn as_url(&self) -> &Url {
        &self.0
    }

    /// Consume the wrapper and return its URL.
    #[must_use]
    pub fn into_url(self) -> Url {
        self.0
    }
}

impl fmt::Debug for SurfpoolRpcUrl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("SurfpoolRpcUrl")
            .field(&self.0.as_str())
            .finish()
    }
}

impl fmt::Display for SurfpoolRpcUrl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for SurfpoolRpcUrl {
    type Err = RpcError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let url = Url::parse(value)
            .map_err(|error| RpcError::identity("connect", format!("invalid RPC URL: {error}")))?;
        Self::new(url)
    }
}

impl TryFrom<Url> for SurfpoolRpcUrl {
    type Error = RpcError;

    fn try_from(url: Url) -> Result<Self, Self::Error> {
        Self::new(url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_explicit_loopback_http_is_accepted() -> Result<(), RpcError> {
        let ipv4: SurfpoolRpcUrl = "http://127.0.0.1:8899".parse()?;
        let ipv6: SurfpoolRpcUrl = "http://[::1]:8899".parse()?;
        assert_eq!(ipv4.as_url().host_str(), Some("127.0.0.1"));
        assert_eq!(ipv6.as_url().host_str(), Some("[::1]"));
        Ok(())
    }

    #[test]
    fn public_dns_and_localhost_names_are_rejected() {
        for unsafe_url in [
            "https://api.mainnet-beta.solana.com",
            "http://api.devnet.solana.com",
            "http://localhost:8899",
            "http://0.0.0.0:8899",
            "http://127.0.0.1",
            "http://127.0.0.1:8899/rpc",
        ] {
            assert!(
                unsafe_url.parse::<SurfpoolRpcUrl>().is_err(),
                "{unsafe_url}"
            );
        }
    }
}

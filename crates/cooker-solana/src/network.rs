//! RPC endpoint validation with a loopback-only default and an explicit public-cluster opt-in.

use std::{fmt, net::IpAddr, str::FromStr};

use url::{Host, Url};

use crate::RpcError;

/// Public Solana cluster an endpoint may be deliberately declared for.
///
/// Constructing one of these values is the only way to leave loopback, and no configuration
/// file, environment variable, or CLI flag can produce it.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PublicCluster {
    /// Solana devnet.
    Devnet,
}

impl PublicCluster {
    /// Stable identifier used in evidence records and store identities.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Devnet => "solana-devnet",
        }
    }

    /// Genesis hash the cluster must report before a gateway is constructed.
    #[must_use]
    pub const fn genesis_hash(self) -> &'static str {
        match self {
            Self::Devnet => "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG",
        }
    }
}

impl fmt::Display for PublicCluster {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Network family a validated endpoint is allowed to address.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RpcEndpointClass {
    /// A local Surfpool process reachable only over an IP loopback interface.
    LoopbackSurfpool,
    /// A named public Solana cluster reachable over the internet.
    PublicCluster(PublicCluster),
}

/// Validated HTTP endpoint carrying the network policy it was validated under.
///
/// [`FromStr`] and [`TryFrom<Url>`], the only paths reachable from parsed configuration, accept
/// loopback addresses exclusively. Addressing a public cluster requires
/// [`RpcEndpoint::public_cluster`] and a [`PublicCluster`] value written in source.
#[derive(Clone, Eq, Hash, PartialEq)]
pub struct RpcEndpoint {
    url: Url,
    class: RpcEndpointClass,
}

impl RpcEndpoint {
    /// Validate an already parsed loopback URL.
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
        reject_shared_url_hazards(&url)?;
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
        Ok(Self {
            url,
            class: RpcEndpointClass::LoopbackSurfpool,
        })
    }

    /// Validate an endpoint that deliberately addresses a named public Solana cluster.
    ///
    /// The caller names the cluster in source, which keeps every configuration-driven code path
    /// loopback-only. The endpoint still has to prove that cluster's genesis hash before a
    /// gateway is constructed.
    ///
    /// # Errors
    ///
    /// Returns [`RpcError`] unless the URL is HTTPS, has a DNS host, and contains no credentials,
    /// path, query, or fragment.
    pub fn public_cluster(url: Url, cluster: PublicCluster) -> Result<Self, RpcError> {
        if url.scheme() != "https" {
            return Err(RpcError::identity(
                "connect",
                "public cluster RPC must use HTTPS",
            ));
        }
        reject_shared_url_hazards(&url)?;
        match url.host() {
            Some(Host::Domain(domain)) if !domain.is_empty() => {}
            _ => {
                return Err(RpcError::identity(
                    "connect",
                    "public cluster RPC host must be a DNS name",
                ));
            }
        }
        Ok(Self {
            url,
            class: RpcEndpointClass::PublicCluster(cluster),
        })
    }

    /// Borrow the validated URL.
    #[must_use]
    pub const fn as_url(&self) -> &Url {
        &self.url
    }

    /// Return the policy this endpoint was validated under.
    #[must_use]
    pub const fn class(&self) -> RpcEndpointClass {
        self.class
    }

    /// Return the named public cluster when this endpoint leaves loopback.
    #[must_use]
    pub const fn public_cluster_target(&self) -> Option<PublicCluster> {
        match self.class {
            RpcEndpointClass::LoopbackSurfpool => None,
            RpcEndpointClass::PublicCluster(cluster) => Some(cluster),
        }
    }

    /// Report whether this endpoint addresses a public cluster.
    #[must_use]
    pub const fn is_public_cluster(&self) -> bool {
        self.public_cluster_target().is_some()
    }

    /// Render the endpoint for evidence.
    ///
    /// Loopback ports are an operator detail and stay redacted. A public cluster URL is a
    /// published, reviewer-checkable fact and is reported verbatim.
    #[must_use]
    pub fn for_evidence(&self) -> String {
        match self.class {
            RpcEndpointClass::LoopbackSurfpool => "http://127.0.0.1:<local>".to_owned(),
            RpcEndpointClass::PublicCluster(_) => self.url.as_str().to_owned(),
        }
    }
}

fn reject_shared_url_hazards(url: &Url) -> Result<(), RpcError> {
    if !url.username().is_empty() || url.password().is_some() {
        return Err(RpcError::identity(
            "connect",
            "RPC URL must not contain credentials",
        ));
    }
    if url.query().is_some() || url.fragment().is_some() || !matches!(url.path(), "" | "/") {
        return Err(RpcError::identity(
            "connect",
            "RPC URL must not contain a path, query, or fragment",
        ));
    }
    Ok(())
}

impl fmt::Debug for RpcEndpoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RpcEndpoint")
            .field("url", &self.url.as_str())
            .field("class", &self.class)
            .finish()
    }
}

impl fmt::Display for RpcEndpoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.url.fmt(formatter)
    }
}

impl FromStr for RpcEndpoint {
    type Err = RpcError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let url = Url::parse(value)
            .map_err(|error| RpcError::identity("connect", format!("invalid RPC URL: {error}")))?;
        Self::new(url)
    }
}

impl TryFrom<Url> for RpcEndpoint {
    type Error = RpcError;

    fn try_from(url: Url) -> Result<Self, Self::Error> {
        Self::new(url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(value: &str) -> Result<Url, RpcError> {
        Url::parse(value).map_err(|error| RpcError::identity("connect", error.to_string()))
    }

    #[test]
    fn only_explicit_loopback_http_is_accepted() -> Result<(), RpcError> {
        let ipv4: RpcEndpoint = "http://127.0.0.1:8899".parse()?;
        let ipv6: RpcEndpoint = "http://[::1]:8899".parse()?;
        assert_eq!(ipv4.as_url().host_str(), Some("127.0.0.1"));
        assert_eq!(ipv6.as_url().host_str(), Some("[::1]"));
        assert_eq!(ipv4.class(), RpcEndpointClass::LoopbackSurfpool);
        assert!(!ipv4.is_public_cluster());
        assert_eq!(ipv4.for_evidence(), "http://127.0.0.1:<local>");
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
            assert!(unsafe_url.parse::<RpcEndpoint>().is_err(), "{unsafe_url}");
        }
    }

    #[test]
    fn public_cluster_requires_an_explicit_cluster_value() -> Result<(), RpcError> {
        let url = parse("https://api.devnet.solana.com")?;
        let endpoint = RpcEndpoint::public_cluster(url.clone(), PublicCluster::Devnet)?;
        assert_eq!(
            endpoint.class(),
            RpcEndpointClass::PublicCluster(PublicCluster::Devnet)
        );
        assert!(endpoint.is_public_cluster());
        assert_eq!(
            endpoint.public_cluster_target().map(PublicCluster::as_str),
            Some("solana-devnet")
        );
        assert_eq!(endpoint.for_evidence(), "https://api.devnet.solana.com/");
        // The same URL stays refused by every configuration-reachable path.
        assert!(RpcEndpoint::new(url).is_err());
        Ok(())
    }

    /// The pinned genesis hash is the whole identity check, so a typo in it would silently
    /// allow the wrong cluster. Mainnet-beta's hash is the one that must never appear here.
    #[test]
    fn devnet_genesis_hash_is_not_the_mainnet_hash() {
        const MAINNET_BETA_GENESIS_HASH: &str = "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d";
        assert_ne!(
            PublicCluster::Devnet.genesis_hash(),
            MAINNET_BETA_GENESIS_HASH
        );
        assert_eq!(
            PublicCluster::Devnet.genesis_hash(),
            "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG"
        );
    }

    #[test]
    fn public_cluster_rejects_plaintext_credentials_and_paths() -> Result<(), RpcError> {
        for unsafe_url in [
            "http://api.devnet.solana.com",
            "https://user:secret@api.devnet.solana.com",
            "https://api.devnet.solana.com/?api-key=1",
            "https://api.devnet.solana.com/rpc",
            "https://127.0.0.1",
        ] {
            let url = parse(unsafe_url)?;
            assert!(
                RpcEndpoint::public_cluster(url, PublicCluster::Devnet).is_err(),
                "{unsafe_url}"
            );
        }
        Ok(())
    }
}

//! Durable public fleet manifest backed by ignored, permission-checked local key files.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
};

use chrono::{DateTime, Utc};
use cooker_core::{AgentId, RunId};
use serde::{Deserialize, Serialize};
use solana_pubkey::Pubkey;

use crate::{
    LocalKeypair, RpcError,
    private_fs::{
        open_private_new, secure_directory, sync_directory, validate_private_permissions,
    },
};

const FLEET_SCHEMA_VERSION: u16 = 1;
const MAX_FLEET_AGENTS: usize = 10_000;
const MAX_MANIFEST_BYTES: u64 = 8 * 1024 * 1024;
const MANIFEST_RELATIVE_PATH: &str = ".surfpool/fleet.json";

/// Public signer binding for one durable agent. Secret bytes remain only in `key_path`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FleetEntry {
    /// Stable zero-based run-local fleet index.
    pub index: u64,
    /// Durable planner and store identity.
    pub agent_id: AgentId,
    /// Base58 public signer address.
    pub public_key: String,
    /// Project-relative ignored key file.
    pub key_path: String,
}

/// Versioned manifest that restores the same agent-to-signer routing after restart.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FleetManifest {
    schema_version: u16,
    run_id: RunId,
    surfnet_id: String,
    created_at: DateTime<Utc>,
    entries: Vec<FleetEntry>,
}

impl FleetManifest {
    /// Generate a new non-overwriting fleet and persist its public manifest.
    ///
    /// # Errors
    ///
    /// Returns [`RpcError`] for invalid bounds or identity, an existing manifest/key, unsafe file
    /// permissions, entropy/key generation failure, incomplete writes, or manifest validation
    /// failure. Errors during normal execution remove keys created by this call.
    pub fn create(
        project_root: impl AsRef<Path>,
        run_id: RunId,
        surfnet_id: impl Into<String>,
        agent_count: usize,
        created_at: DateTime<Utc>,
    ) -> Result<Self, RpcError> {
        let surfnet_id = surfnet_id.into();
        validate_header(&surfnet_id, agent_count)?;
        let root = canonical_root(project_root.as_ref(), "createFleet")?;
        let manifest_path = root.join(MANIFEST_RELATIVE_PATH);
        if manifest_path.exists() {
            return Err(RpcError::invalid_input(
                "createFleet",
                format!("fleet manifest already exists: {}", manifest_path.display()),
            ));
        }
        let surfpool_dir = root.join(".surfpool");
        let keys_dir = surfpool_dir.join("keys");
        fs::create_dir_all(&keys_dir).map_err(|error| {
            RpcError::invalid_input(
                "createFleet",
                format!("cannot create fleet directory: {error}"),
            )
        })?;
        secure_directory(&surfpool_dir, "createFleet")?;
        secure_directory(&keys_dir, "createFleet")?;

        let mut planned = Vec::with_capacity(agent_count);
        for raw_index in 0..agent_count {
            let index = u64::try_from(raw_index).map_err(|error| {
                RpcError::invalid_input("createFleet", format!("fleet index overflow: {error}"))
            })?;
            let agent_id = AgentId::derive(run_id, index);
            let relative = format!(".surfpool/keys/agent-{index:05}-{agent_id}.json");
            let absolute = root.join(&relative);
            if absolute.exists() {
                return Err(RpcError::invalid_input(
                    "createFleet",
                    format!("fleet key already exists: {}", absolute.display()),
                ));
            }
            planned.push((index, agent_id, relative, absolute));
        }

        let mut generated = Vec::with_capacity(agent_count);
        let result = (|| {
            let mut entries = Vec::with_capacity(agent_count);
            for (index, agent_id, relative, absolute) in &planned {
                let signer = LocalKeypair::generate(&root, absolute)?;
                generated.push(absolute.clone());
                entries.push(FleetEntry {
                    index: *index,
                    agent_id: *agent_id,
                    public_key: signer.pubkey().to_string(),
                    key_path: relative.clone(),
                });
            }
            let manifest = Self {
                schema_version: FLEET_SCHEMA_VERSION,
                run_id,
                surfnet_id,
                created_at,
                entries,
            };
            manifest.validate(&root)?;
            write_manifest(&manifest_path, &manifest)?;
            sync_directory(&surfpool_dir, "createFleet")?;
            Ok(manifest)
        })();
        if result.is_err() {
            for path in generated {
                let _ = fs::remove_file(path);
            }
        }
        result
    }

    /// Load and fully validate the fixed ignored fleet manifest and every signer binding.
    ///
    /// # Errors
    ///
    /// Returns [`RpcError`] for a missing, oversized, symlinked, permission-unsafe, malformed, or
    /// internally inconsistent manifest, or when any key file does not match its public binding.
    pub fn load(project_root: impl AsRef<Path>) -> Result<Self, RpcError> {
        let root = canonical_root(project_root.as_ref(), "loadFleet")?;
        let manifest = Self::read_public(&root)?;
        manifest.validate(&root)?;
        Ok(manifest)
    }

    /// Load and validate only the public manifest without opening any signer file.
    ///
    /// This is the only fleet loader suitable for dry-run and status operations. Execute paths
    /// must use [`Self::load`] or [`Self::load_signers`] after Surfpool identity preflight.
    ///
    /// # Errors
    ///
    /// Returns [`RpcError`] for a missing, unsafe, malformed, or structurally invalid manifest.
    pub fn load_public(project_root: impl AsRef<Path>) -> Result<Self, RpcError> {
        let root = canonical_root(project_root.as_ref(), "loadFleetPublic")?;
        Self::read_public(&root)
    }

    fn read_public(root: &Path) -> Result<Self, RpcError> {
        let path = root.join(MANIFEST_RELATIVE_PATH);
        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            RpcError::invalid_input(
                "loadFleet",
                format!("fleet manifest is unavailable: {error}"),
            )
        })?;
        if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
            return Err(RpcError::invalid_input(
                "loadFleet",
                "fleet manifest must be a regular non-symlink file",
            ));
        }
        validate_private_permissions(&metadata, "loadFleet")?;
        if metadata.len() == 0 || metadata.len() > MAX_MANIFEST_BYTES {
            return Err(RpcError::invalid_input(
                "loadFleet",
                "fleet manifest size is invalid",
            ));
        }
        let mut file = File::open(&path).map_err(|error| {
            RpcError::invalid_input("loadFleet", format!("cannot open fleet manifest: {error}"))
        })?;
        let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
        file.read_to_end(&mut bytes).map_err(|error| {
            RpcError::invalid_input("loadFleet", format!("cannot read fleet manifest: {error}"))
        })?;
        let manifest: Self = serde_json::from_slice(&bytes).map_err(|error| {
            RpcError::invalid_input("loadFleet", format!("invalid fleet manifest JSON: {error}"))
        })?;
        manifest.validate_structure()?;
        Ok(manifest)
    }

    /// Durable run bound to this fleet.
    #[must_use]
    pub const fn run_id(&self) -> RunId {
        self.run_id
    }

    /// Immutable fleet creation time used as the deterministic epoch for auxiliary workflows.
    #[must_use]
    pub const fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }

    /// Surfnet identity that must match both config and store.
    #[must_use]
    pub fn surfnet_id(&self) -> &str {
        &self.surfnet_id
    }

    /// Stable public fleet entries in index order.
    #[must_use]
    pub fn entries(&self) -> &[FleetEntry] {
        &self.entries
    }

    /// Load all signer files and return durable agent routing without exposing secret bytes.
    ///
    /// # Errors
    ///
    /// Returns [`RpcError`] when the manifest or any signer no longer validates.
    pub fn load_signers(
        &self,
        project_root: impl AsRef<Path>,
    ) -> Result<BTreeMap<AgentId, Arc<LocalKeypair>>, RpcError> {
        let root = canonical_root(project_root.as_ref(), "loadFleet")?;
        self.validate(&root)?;
        let mut signers = BTreeMap::new();
        for entry in &self.entries {
            let signer = Arc::new(LocalKeypair::load(&root, root.join(&entry.key_path))?);
            signers.insert(entry.agent_id, signer);
        }
        Ok(signers)
    }

    fn validate(&self, root: &Path) -> Result<(), RpcError> {
        self.validate_structure()?;
        for entry in &self.entries {
            let public_key = Pubkey::from_str(&entry.public_key).map_err(|error| {
                RpcError::invalid_input("loadFleet", format!("invalid public key: {error}"))
            })?;
            let signer = LocalKeypair::load(root, root.join(&entry.key_path))?;
            if signer.pubkey() != public_key {
                return Err(RpcError::invalid_input(
                    "loadFleet",
                    format!("signer mismatch for agent {}", entry.agent_id),
                ));
            }
        }
        Ok(())
    }

    fn validate_structure(&self) -> Result<(), RpcError> {
        if self.schema_version != FLEET_SCHEMA_VERSION {
            return Err(RpcError::invalid_input(
                "loadFleet",
                format!("unsupported fleet schema version {}", self.schema_version),
            ));
        }
        validate_header(&self.surfnet_id, self.entries.len())?;
        let mut agents = BTreeSet::new();
        let mut public_keys = BTreeSet::new();
        for (position, entry) in self.entries.iter().enumerate() {
            let expected_index = u64::try_from(position).map_err(|error| {
                RpcError::invalid_input("loadFleet", format!("fleet index overflow: {error}"))
            })?;
            if entry.index != expected_index
                || entry.agent_id != AgentId::derive(self.run_id, expected_index)
            {
                return Err(RpcError::invalid_input(
                    "loadFleet",
                    "fleet entries are not in canonical run-index order",
                ));
            }
            if !agents.insert(entry.agent_id) || !public_keys.insert(entry.public_key.clone()) {
                return Err(RpcError::invalid_input(
                    "loadFleet",
                    "fleet contains duplicate agent or signer identities",
                ));
            }
            let expected_key_path = format!(
                ".surfpool/keys/agent-{expected_index:05}-{}.json",
                entry.agent_id
            );
            if entry.key_path != expected_key_path {
                return Err(RpcError::invalid_input(
                    "loadFleet",
                    "fleet key path is not canonical",
                ));
            }
            Pubkey::from_str(&entry.public_key).map_err(|error| {
                RpcError::invalid_input("loadFleet", format!("invalid public key: {error}"))
            })?;
        }
        Ok(())
    }
}

fn validate_header(surfnet_id: &str, agent_count: usize) -> Result<(), RpcError> {
    if surfnet_id.trim().is_empty() {
        return Err(RpcError::invalid_input(
            "createFleet",
            "Surfnet identity cannot be empty",
        ));
    }
    if agent_count == 0 || agent_count > MAX_FLEET_AGENTS {
        return Err(RpcError::invalid_input(
            "createFleet",
            format!("fleet size must be in 1..={MAX_FLEET_AGENTS}"),
        ));
    }
    Ok(())
}

fn canonical_root(root: &Path, method: &'static str) -> Result<PathBuf, RpcError> {
    root.canonicalize()
        .map_err(|error| RpcError::invalid_input(method, format!("invalid project root: {error}")))
}

fn write_manifest(path: &Path, manifest: &FleetManifest) -> Result<(), RpcError> {
    let mut bytes = serde_json::to_vec_pretty(manifest).map_err(|error| {
        RpcError::invalid_input("createFleet", format!("cannot serialize fleet: {error}"))
    })?;
    bytes.push(b'\n');
    let mut file = open_private_new(path, "createFleet")?;
    if let Err(error) = file.write_all(&bytes).and_then(|()| file.sync_all()) {
        let _ = fs::remove_file(path);
        return Err(RpcError::invalid_input(
            "createFleet",
            format!("cannot persist fleet manifest: {error}"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use tempfile::TempDir;
    use uuid::Uuid;

    use super::*;

    #[cfg(unix)]
    #[test]
    fn fleet_round_trip_binds_distinct_private_signers() -> Result<(), Box<dyn std::error::Error>> {
        let root = TempDir::new()?;
        let run_id = RunId(Uuid::from_u128(77));
        let created_at = Utc
            .with_ymd_and_hms(2026, 7, 17, 12, 0, 0)
            .single()
            .ok_or("invalid fixture time")?;
        let manifest = FleetManifest::create(root.path(), run_id, "fleet-test", 3, created_at)?;
        let loaded = FleetManifest::load(root.path())?;
        assert_eq!(loaded, manifest);
        assert_eq!(loaded.entries().len(), 3);
        assert_eq!(loaded.load_signers(root.path())?.len(), 3);
        assert_eq!(
            loaded
                .entries()
                .iter()
                .map(|entry| &entry.public_key)
                .collect::<BTreeSet<_>>()
                .len(),
            3
        );
        assert!(FleetManifest::create(root.path(), run_id, "fleet-test", 3, created_at).is_err());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn tampered_public_binding_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
        let root = TempDir::new()?;
        let run_id = RunId(Uuid::from_u128(78));
        let created_at = Utc
            .with_ymd_and_hms(2026, 7, 17, 12, 0, 0)
            .single()
            .ok_or("invalid fixture time")?;
        let mut manifest = FleetManifest::create(root.path(), run_id, "fleet-test", 2, created_at)?;
        manifest.entries[1].public_key = manifest.entries[0].public_key.clone();
        let path = root.path().join(MANIFEST_RELATIVE_PATH);
        fs::remove_file(&path)?;
        write_manifest(&path, &manifest)?;
        assert!(FleetManifest::load(root.path()).is_err());
        Ok(())
    }
}

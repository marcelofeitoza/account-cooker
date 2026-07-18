//! Secure loading for ignored local Surfpool signer files.

use std::{
    fmt,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use zeroize::Zeroize;

use crate::RpcError;

const MAX_KEYPAIR_FILE_BYTES: u64 = 4_096;

/// Locally loaded signer restricted to `<project>/.surfpool/keys`.
pub struct LocalKeypair {
    path: PathBuf,
    keypair: Arc<Keypair>,
}

impl LocalKeypair {
    /// Generate a new signer with OS entropy in the ignored Surfpool key directory.
    ///
    /// The destination must be a direct child of `<project>/.surfpool/keys` and must not already
    /// exist. The file is created with mode `0600`, synchronized before return, and immediately
    /// reloaded through the same validation path used for existing signers.
    ///
    /// # Errors
    ///
    /// Returns [`RpcError`] when the project root or destination is invalid, secure Unix file
    /// creation is unavailable, the destination exists, or writing and validating the key fails.
    pub fn generate(
        project_root: impl AsRef<Path>,
        path: impl AsRef<Path>,
    ) -> Result<Self, RpcError> {
        let root = project_root.as_ref().canonicalize().map_err(|error| {
            RpcError::invalid_input("generateKeypair", format!("invalid project root: {error}"))
        })?;
        let allowed_dir = root.join(".surfpool/keys");
        fs::create_dir_all(&allowed_dir).map_err(|error| {
            RpcError::invalid_input(
                "generateKeypair",
                format!("cannot create Surfpool key directory: {error}"),
            )
        })?;
        set_private_directory_permissions(&allowed_dir)?;
        let allowed_dir = allowed_dir.canonicalize().map_err(|error| {
            RpcError::invalid_input(
                "generateKeypair",
                format!("invalid Surfpool key directory: {error}"),
            )
        })?;
        let requested = if path.as_ref().is_absolute() {
            path.as_ref().to_path_buf()
        } else {
            root.join(path)
        };
        let parent = requested.parent().ok_or_else(|| {
            RpcError::invalid_input("generateKeypair", "keypair destination has no parent")
        })?;
        let canonical_parent = parent.canonicalize().map_err(|error| {
            RpcError::invalid_input(
                "generateKeypair",
                format!("invalid keypair parent directory: {error}"),
            )
        })?;
        if canonical_parent != allowed_dir || requested.file_name().is_none() {
            return Err(RpcError::invalid_input(
                "generateKeypair",
                "generated keypair must be a direct child of .surfpool/keys",
            ));
        }

        let keypair = Keypair::new();
        let mut serialized =
            serde_json::to_vec(keypair.to_bytes().as_slice()).map_err(|error| {
                RpcError::invalid_input(
                    "generateKeypair",
                    format!("cannot serialize keypair: {error}"),
                )
            })?;
        let write_result = (|| -> Result<(), std::io::Error> {
            let mut file = open_private_new(&requested)?;
            file.write_all(&serialized)?;
            file.write_all(b"\n")?;
            file.sync_all()?;
            Ok(())
        })();
        serialized.zeroize();
        if let Err(error) = write_result {
            let _ = fs::remove_file(&requested);
            return Err(RpcError::invalid_input(
                "generateKeypair",
                format!("cannot create keypair: {error}"),
            ));
        }
        sync_directory(&allowed_dir)?;
        Self::load(&root, &requested)
    }

    /// Load and validate a JSON keypair from the ignored Surfpool key directory.
    ///
    /// # Errors
    ///
    /// Returns [`RpcError`] when the path escapes `.surfpool/keys`, is a symlink or non-file,
    /// has unsafe Unix permissions, exceeds the size limit, or contains invalid keypair bytes.
    pub fn load(project_root: impl AsRef<Path>, path: impl AsRef<Path>) -> Result<Self, RpcError> {
        let root = project_root.as_ref().canonicalize().map_err(|error| {
            RpcError::invalid_input("loadKeypair", format!("invalid project root: {error}"))
        })?;
        let allowed_dir = root
            .join(".surfpool/keys")
            .canonicalize()
            .map_err(|error| {
                RpcError::invalid_input(
                    "loadKeypair",
                    format!("Surfpool key directory is unavailable: {error}"),
                )
            })?;

        let link_metadata = fs::symlink_metadata(path.as_ref()).map_err(|error| {
            RpcError::invalid_input("loadKeypair", format!("keypair is unavailable: {error}"))
        })?;
        if link_metadata.file_type().is_symlink() || !link_metadata.file_type().is_file() {
            return Err(RpcError::invalid_input(
                "loadKeypair",
                "keypair must be a regular non-symlink file",
            ));
        }

        let canonical_path = path.as_ref().canonicalize().map_err(|error| {
            RpcError::invalid_input("loadKeypair", format!("invalid keypair path: {error}"))
        })?;
        if !canonical_path.starts_with(&allowed_dir) {
            return Err(RpcError::invalid_input(
                "loadKeypair",
                "keypair must be stored under .surfpool/keys",
            ));
        }

        let file = File::open(&canonical_path).map_err(|error| {
            RpcError::invalid_input("loadKeypair", format!("cannot open keypair: {error}"))
        })?;
        let metadata = file.metadata().map_err(|error| {
            RpcError::invalid_input("loadKeypair", format!("cannot inspect keypair: {error}"))
        })?;
        if metadata.len() == 0 || metadata.len() > MAX_KEYPAIR_FILE_BYTES {
            return Err(RpcError::invalid_input(
                "loadKeypair",
                "keypair file size is invalid",
            ));
        }
        validate_permissions(&metadata)?;

        let mut bytes: Vec<u8> = serde_json::from_reader(file).map_err(|error| {
            RpcError::invalid_input("loadKeypair", format!("invalid keypair JSON: {error}"))
        })?;
        let keypair_result = Keypair::try_from(bytes.as_slice());
        bytes.zeroize();
        let keypair = keypair_result.map_err(|error| {
            RpcError::invalid_input("loadKeypair", format!("invalid keypair bytes: {error}"))
        })?;

        Ok(Self {
            path: canonical_path,
            keypair: Arc::new(keypair),
        })
    }

    /// Return the public address without exposing secret bytes.
    #[must_use]
    pub fn pubkey(&self) -> Pubkey {
        self.keypair.pubkey()
    }

    /// Return the validated canonical file path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn keypair(&self) -> &Keypair {
        &self.keypair
    }

    #[cfg(test)]
    pub(crate) fn from_keypair_for_test(keypair: Keypair) -> Self {
        Self {
            path: PathBuf::from("<test>"),
            keypair: Arc::new(keypair),
        }
    }
}

#[cfg(unix)]
fn open_private_new(path: &Path) -> std::io::Result<File> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn open_private_new(_path: &Path) -> std::io::Result<File> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "secure keypair creation requires Unix",
    ))
}

#[cfg(unix)]
fn set_private_directory_permissions(path: &Path) -> Result<(), RpcError> {
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|error| {
        RpcError::invalid_input(
            "generateKeypair",
            format!("cannot secure key directory: {error}"),
        )
    })
}

#[cfg(not(unix))]
fn set_private_directory_permissions(_path: &Path) -> Result<(), RpcError> {
    Err(RpcError::invalid_input(
        "generateKeypair",
        "secure keypair creation requires Unix",
    ))
}

fn sync_directory(path: &Path) -> Result<(), RpcError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| {
            RpcError::invalid_input(
                "generateKeypair",
                format!("cannot synchronize key directory: {error}"),
            )
        })
}

impl fmt::Debug for LocalKeypair {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalKeypair")
            .field("path", &self.path)
            .field("pubkey", &self.pubkey().to_string())
            .finish_non_exhaustive()
    }
}

#[cfg(unix)]
fn validate_permissions(metadata: &fs::Metadata) -> Result<(), RpcError> {
    let mode = metadata.permissions().mode();
    if mode & 0o077 != 0 {
        return Err(RpcError::invalid_input(
            "loadKeypair",
            "keypair permissions must deny group and other access",
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_permissions(_metadata: &fs::Metadata) -> Result<(), RpcError> {
    Err(RpcError::invalid_input(
        "loadKeypair",
        "secure keypair permission validation requires Unix",
    ))
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::TempDir;

    use super::*;

    fn write_keypair(mode: u32) -> Result<(TempDir, PathBuf, Pubkey), Box<dyn std::error::Error>> {
        let temp = TempDir::new()?;
        let key_dir = temp.path().join(".surfpool/keys");
        fs::create_dir_all(&key_dir)?;
        let keypair = Keypair::new();
        let pubkey = keypair.pubkey();
        let path = key_dir.join("agent.json");
        let mut file = File::create(&path)?;
        serde_json::to_writer(&mut file, &keypair.to_bytes().as_slice())?;
        file.flush()?;
        #[cfg(unix)]
        fs::set_permissions(&path, fs::Permissions::from_mode(mode))?;
        Ok((temp, path, pubkey))
    }

    #[test]
    fn valid_ignored_keypair_loads_without_secret_debug_output()
    -> Result<(), Box<dyn std::error::Error>> {
        let (temp, path, expected_pubkey) = write_keypair(0o600)?;
        let signer = LocalKeypair::load(temp.path(), &path)?;
        assert_eq!(signer.pubkey(), expected_pubkey);
        let debug = format!("{signer:?}");
        assert!(debug.contains(&expected_pubkey.to_string()));
        assert!(!debug.contains(&serde_json::to_string(
            signer.keypair.to_bytes().as_slice()
        )?));
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn group_readable_keypair_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
        let (temp, path, _) = write_keypair(0o640)?;
        let error = LocalKeypair::load(temp.path(), path);
        assert!(error.is_err());
        Ok(())
    }

    #[test]
    fn keypair_outside_surfpool_directory_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
        let (temp, path, _) = write_keypair(0o600)?;
        let outside = temp.path().join("outside.json");
        fs::copy(path, &outside)?;
        #[cfg(unix)]
        fs::set_permissions(&outside, fs::Permissions::from_mode(0o600))?;
        assert!(LocalKeypair::load(temp.path(), outside).is_err());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn generated_keypair_is_private_loadable_and_never_overwritten()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = TempDir::new()?;
        let path = temp.path().join(".surfpool/keys/agent.json");
        let signer = LocalKeypair::generate(temp.path(), &path)?;
        assert_eq!(fs::metadata(&path)?.permissions().mode() & 0o777, 0o600);
        assert_eq!(
            LocalKeypair::load(temp.path(), &path)?.pubkey(),
            signer.pubkey()
        );
        assert!(LocalKeypair::generate(temp.path(), &path).is_err());
        assert!(LocalKeypair::generate(temp.path(), temp.path().join("outside.json")).is_err());
        Ok(())
    }
}

//! Same-directory temporary-file persistence for complete artifacts only.

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use cooker_core::TraceEvent;
use tempfile::NamedTempFile;

/// Size and digest of a stable trace JSONL stream.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TraceDigest {
    pub bytes: u64,
    pub events: usize,
    pub blake3: String,
}

/// Atomically persist a set of files after a complete preflight and staging pass.
pub(crate) fn write_artifacts(artifacts: Vec<(PathBuf, Vec<u8>)>, force: bool) -> Result<()> {
    if artifacts.is_empty() {
        return Ok(());
    }
    if !force {
        for (path, _) in &artifacts {
            if path.exists() {
                bail!(
                    "refusing to overwrite {}; pass --force to replace it",
                    path.display()
                );
            }
        }
    }

    let mut staged = Vec::with_capacity(artifacts.len());
    for (path, bytes) in artifacts {
        let parent = usable_parent(&path);
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create output directory {}", parent.display()))?;
        let mut temporary = NamedTempFile::new_in(parent)
            .with_context(|| format!("failed to stage output for {}", path.display()))?;
        temporary
            .write_all(&bytes)
            .and_then(|()| temporary.flush())
            .and_then(|()| temporary.as_file().sync_all())
            .with_context(|| format!("failed to stage complete output for {}", path.display()))?;
        staged.push((temporary, path));
    }

    for (temporary, path) in staged {
        persist(temporary, &path, force)?;
    }
    Ok(())
}

/// Serialize and atomically persist one trace without materializing the JSONL corpus.
pub(crate) fn write_trace_atomic(
    path: &Path,
    events: &[TraceEvent],
    force: bool,
) -> Result<TraceDigest> {
    if !force && path.exists() {
        bail!(
            "refusing to overwrite {}; pass --force to replace it",
            path.display()
        );
    }
    let parent = usable_parent(path);
    fs::create_dir_all(parent)
        .with_context(|| format!("failed to create output directory {}", parent.display()))?;
    let mut temporary = NamedTempFile::new_in(parent)
        .with_context(|| format!("failed to stage output for {}", path.display()))?;
    let digest = stream_trace(events, &mut temporary)
        .with_context(|| format!("failed to stage trace {}", path.display()))?;
    temporary
        .flush()
        .and_then(|()| temporary.as_file().sync_all())
        .with_context(|| format!("failed to sync trace {}", path.display()))?;
    persist(temporary, path, force)?;
    Ok(digest)
}

/// Compute the exact JSONL digest for a replay without retaining serialized rows.
pub(crate) fn trace_digest(events: &[TraceEvent]) -> Result<TraceDigest> {
    stream_trace(events, &mut std::io::sink())
}

/// Reject an existing destination before an expensive offline computation.
pub(crate) fn preflight(paths: &[PathBuf], force: bool) -> Result<()> {
    if force {
        return Ok(());
    }
    for path in paths {
        if path.exists() {
            bail!(
                "refusing to overwrite {}; pass --force to replace it",
                path.display()
            );
        }
    }
    Ok(())
}

fn usable_parent(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn stream_trace(events: &[TraceEvent], writer: &mut impl Write) -> Result<TraceDigest> {
    let mut hasher = blake3::Hasher::new();
    let mut bytes = 0_u64;
    for event in events {
        let row = serde_json::to_vec(event).context("failed to serialize trace event")?;
        writer
            .write_all(&row)
            .and_then(|()| writer.write_all(b"\n"))
            .context("failed to write trace event")?;
        hasher.update(&row);
        hasher.update(b"\n");
        let row_bytes = u64::try_from(row.len()).context("trace row size overflow")?;
        bytes = bytes
            .checked_add(row_bytes)
            .and_then(|value| value.checked_add(1))
            .context("trace size overflow")?;
    }
    Ok(TraceDigest {
        bytes,
        events: events.len(),
        blake3: hasher.finalize().to_hex().to_string(),
    })
}

fn persist(temporary: NamedTempFile, path: &Path, force: bool) -> Result<()> {
    let persisted = if force {
        temporary.persist(path)
    } else {
        temporary.persist_noclobber(path)
    };
    persisted.map_err(|error| {
        anyhow::anyhow!(
            "failed to atomically persist {}: {}",
            path.display(),
            error.error
        )
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn no_clobber_is_default_and_force_replaces() -> Result<()> {
        let directory = tempdir()?;
        let path = directory.path().join("result.txt");
        write_artifacts(vec![(path.clone(), b"first".to_vec())], false)?;
        assert!(write_artifacts(vec![(path.clone(), b"second".to_vec())], false).is_err());
        assert_eq!(fs::read(&path)?, b"first");
        write_artifacts(vec![(path.clone(), b"second".to_vec())], true)?;
        assert_eq!(fs::read(path)?, b"second");
        Ok(())
    }
}

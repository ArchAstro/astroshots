//! Immutable, target-specific Node tools embedded in the engine.
use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

pub const TARGET: &str = env!("ASTROSHOT_HELPER_TARGET");
pub const DIGEST: &str = env!("ASTROSHOT_HELPER_DIGEST");
const ARCHIVE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/node-helper.tar.gz"));

/// Materialize the complete helper offline. Concurrent first use publishes only
/// complete directories. Interrupted staging directories are never selected.
/// Corruption is reported instead of rewriting files used by running helpers.
pub fn materialize(cache: &Path) -> io::Result<PathBuf> {
    let parent = cache.join("astroshot/node-helper").join(TARGET);
    let destination = parent.join(DIGEST);
    if destination.exists() {
        verify(&destination)?;
        return Ok(destination.join("helper.mjs"));
    }
    fs::create_dir_all(&parent)?;
    let temporary = tempfile::Builder::new()
        .prefix(".staging-")
        .tempdir_in(&parent)?;
    tar::Archive::new(GzDecoder::new(ARCHIVE)).unpack(temporary.path())?;
    verify(temporary.path())?;
    match fs::rename(temporary.path(), &destination) {
        Ok(()) => {}
        Err(_) if destination.exists() => verify(&destination)?,
        Err(error) => return Err(error),
    }
    Ok(destination.join("helper.mjs"))
}

fn verify(root: &Path) -> io::Result<()> {
    let mut archive = tar::Archive::new(GzDecoder::new(ARCHIVE));
    for entry in archive.entries()? {
        let mut entry = entry?;
        let relative = entry.path()?.into_owned();
        let file = root.join(&relative);
        // Reject replaced symlinks, including parent directories, before reading.
        let mut component = root.to_path_buf();
        for part in relative.components() {
            component.push(part);
            if fs::symlink_metadata(&component).map_or(true, |m| m.file_type().is_symlink()) {
                return Err(corrupt(root, &relative));
            }
        }
        let bytes = fs::read(&file).map_err(|_| corrupt(root, &relative))?;
        let mut expected = Vec::new();
        entry.read_to_end(&mut expected)?;
        if Sha256::digest(&bytes) != Sha256::digest(&expected) {
            return Err(corrupt(root, &relative));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if entry.header().mode()? & 0o111 != 0
                && fs::metadata(&file)?.permissions().mode() & 0o111 == 0
            {
                return Err(corrupt(root, &relative));
            }
        }
    }
    Ok(())
}

fn corrupt(root: &Path, file: &Path) -> io::Error {
    io::Error::other(format!(
        "Astroshots helper cache is incomplete or corrupt at {} ({}). Stop captures, remove this cache directory, and retry; it will be restored offline.",
        root.display(),
        file.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn concurrent_first_use_publishes_complete_helper_and_detects_corruption() {
        let cache = tempfile::tempdir().unwrap();
        let paths = std::thread::scope(|scope| {
            let jobs: Vec<_> = (0..4)
                .map(|_| scope.spawn(|| materialize(cache.path()).unwrap()))
                .collect();
            jobs.into_iter()
                .map(|job| job.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert!(paths.iter().all(|path| path == &paths[0]));
        assert_eq!(materialize(cache.path()).unwrap(), paths[0]);
        let dependency = paths[0]
            .parent()
            .unwrap()
            .join("node_modules/tsx/package.json");
        fs::write(dependency, "damaged").unwrap();
        let error = materialize(cache.path()).unwrap_err().to_string();
        assert!(
            error.contains("corrupt") && error.contains("tsx/package.json"),
            "{error}"
        );
    }
    #[test]
    fn interrupted_staging_is_not_selected() {
        let cache = tempfile::tempdir().unwrap();
        let stale = cache
            .path()
            .join("astroshot/node-helper")
            .join(TARGET)
            .join(".staging-interrupted");
        fs::create_dir_all(&stale).unwrap();
        fs::write(stale.join("helper.mjs"), "partial").unwrap();
        let helper = materialize(cache.path()).unwrap();
        assert!(helper.parent().unwrap().ends_with(DIGEST));
        assert_eq!(
            fs::read_to_string(helper).unwrap(),
            super::super::node_helper::EMBEDDED_HELPER
        );
    }
}

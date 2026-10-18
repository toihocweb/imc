//! Port of `main/proxy/lib/atomic_file.py`.
//!
//! Write-tmp-then-rename: every config/state write in the Rust tree goes
//! through here, exactly like the Python version.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Port of `atomic_file.TEMP_FILE_RANDOM_BYTES`.
pub const TEMP_FILE_RANDOM_BYTES: usize = 8;

/// Port of `atomic_file.fsync_directory` — best effort; failures ignored, same
/// as the Python/TS versions.
pub fn fsync_directory(directory: &Path) {
    if cfg!(windows) {
        return;
    }
    if let Ok(dir) = fs::File::open(directory) {
        let _ = dir.sync_all();
    }
}

/// Temp path built exactly like Python: `.{name}.{pid}.{hex}.tmp` inside the
/// target directory, so the rename stays on one filesystem.
pub fn temp_path_for(target: &Path) -> PathBuf {
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let pid = std::process::id();
    let hex = random_hex(TEMP_FILE_RANDOM_BYTES);
    target
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!(".{name}.{pid}.{hex}.tmp"))
}

/// Port of `secrets.token_hex(n)` — n random bytes as lowercase hex.
pub fn random_hex(bytes: usize) -> String {
    digest_hex(bytes)
}

fn digest_hex(bytes: usize) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id() as u64;
    let digest = crate::utils::sha256_parts(&[
        &nanos.to_le_bytes(),
        &pid.to_le_bytes(),
        &counter.to_le_bytes(),
    ]);
    digest
        .iter()
        .take(bytes)
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Port of `atomic_file.write_file_atomically`.
pub fn write_file_atomically(file_path: &str, content: &str) -> Result<()> {
    write_file_atomically_path(Path::new(file_path), content)
}

pub fn write_file_atomically_path(target: &Path, content: &str) -> Result<()> {
    let directory = target.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(directory).with_context(|| format!("mkdir {}", directory.display()))?;

    let temp_path = temp_path_for(target);

    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }

    let mut file = match opts.open(&temp_path) {
        Ok(f) => f,
        Err(_) => {
            // Stale temp file with the same (pid, counter, ts) — vanishingly
            // unlikely, but never clobber an existing path: pick a new name.
            let temp_path = temp_path_for(target);
            let mut retry = fs::OpenOptions::new();
            retry.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                retry.mode(0o600);
            }
            let mut file = retry
                .open(&temp_path)
                .with_context(|| format!("open {}", temp_path.display()))?;
            return finish_write(target, directory, temp_path, &mut file, content);
        }
    };

    finish_write(target, directory, temp_path, &mut file, content)
}

fn finish_write(
    target: &Path,
    directory: &Path,
    temp_path: PathBuf,
    file: &mut fs::File,
    content: &str,
) -> Result<()> {
    let result = (|| -> Result<()> {
        file.write_all(content.as_bytes())
            .with_context(|| format!("write {}", temp_path.display()))?;
        file.sync_all()
            .with_context(|| format!("fsync {}", temp_path.display()))?;
        fs::rename(&temp_path, target).with_context(|| {
            format!("rename {} -> {}", temp_path.display(), target.display())
        })?;
        Ok(())
    })();

    match result {
        Ok(()) => {
            fsync_directory(directory);
            Ok(())
        }
        Err(e) => {
            let _ = fs::remove_file(&temp_path);
            Err(e)
        }
    }
}
use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use betterlite_config::loader::EnvironmentLookup;
use betterlite_logging::private_json::files::PrivateFiles;
use sha2::{Digest as _, Sha256};
use soft_canonicalize::soft_canonicalize;
use tokio::fs::OpenOptions;
use tokio::time;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;

use super::{Error, SetupResult, Target, settings_path_for};

pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

#[must_use]
pub struct SetupLocksGuard {
    _files: Vec<File>,
} // dropping these closes each descriptor, releasing its OS-level lock

pub fn lock_path_for(target: Target, environment: &dyn EnvironmentLookup) -> SetupResult<PathBuf> {
    let configured = environment.system_home();
    let home = configured.ok_or(Error::Path(target))?;
    let settings_path = settings_path_for(target, environment).ok_or(Error::Path(target))?;
    let resolved = soft_canonicalize(&settings_path)
        .map_err(|source| Error::ResolveSettings(target, settings_path.clone(), source))?;
    let mut digest = Sha256::new();
    digest.update(target.to_string().as_bytes());
    digest.update(b":");
    digest.update(resolved.as_os_str().as_encoded_bytes());
    let name = format!("{}.lock", hex::encode(digest.finalize()));
    Ok(home.join(".litellm").join("setup-locks").join(name))
}

pub async fn setup_locks(
    targets: &[Target],
    environment: &(dyn EnvironmentLookup + Send + Sync),
    clock: &dyn Clock,
) -> SetupResult<SetupLocksGuard> {
    let files_access = PrivateFiles::default();
    let mut files = Vec::new();
    for target in BTreeSet::from_iter(targets.iter().copied()) {
        let path = lock_path_for(target, environment)?;
        let file = open_lock_file(&path, &files_access).await?;
        files.push(acquire_lock(&path, file, clock).await?);
    }
    Ok(SetupLocksGuard { _files: files })
}

async fn acquire_lock(path: &Path, file: File, clock: &dyn Clock) -> SetupResult<File> {
    let started = clock.now();
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(fs::TryLockError::WouldBlock) => {
                let now = clock.now();
                let elapsed = now.saturating_duration_since(started);
                if elapsed >= Duration::from_secs(10) {
                    return Err(Error::Locked);
                }
                time::sleep(Duration::from_millis(10)).await;
            }
            Err(fs::TryLockError::Error(source)) => {
                return Err(Error::LockIo(path.to_owned(), source));
            }
        }
    }
}

async fn open_lock_file(path: &Path, files: &PrivateFiles) -> SetupResult<File> {
    let Some(parent) = path.parent() else {
        return Err(Error::LockIo(
            path.to_owned(),
            io::Error::from(io::ErrorKind::InvalidInput),
        ));
    };
    files
        .ensure_private_dir(parent)
        .await
        .map_err(|source| Error::LockPrepare(path.to_owned(), source))?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    options.mode(0o600);
    let file = options
        .open(path)
        .await
        .map_err(|source| Error::LockIo(path.to_owned(), source))?;
    #[cfg(unix)]
    {
        let metadata = file
            .metadata()
            .await
            .map_err(|source| Error::LockIo(path.to_owned(), source))?;
        if metadata.permissions().mode() & 0o077 != 0 {
            file.set_permissions(fs::Permissions::from_mode(0o600))
                .await
                .map_err(|source| Error::LockIo(path.to_owned(), source))?;
        }
    }
    Ok(file.into_std().await)
}

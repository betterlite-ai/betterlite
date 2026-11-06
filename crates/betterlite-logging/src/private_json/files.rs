use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::task::JoinSet;

use self::blocking::{
    commit_blocking, discard_blocking, ensure_private_dir_blocking, overwrite_blocking,
    stage_blocking,
};
use super::error::PrivateJsonError;

mod blocking;
mod local;

pub const PRIVATE_DIR_MODE: u32 = 0o700;
pub const PRIVATE_FILE_MODE: u32 = 0o600;
pub const MAX_CONCURRENT_FILE_OPERATIONS: usize = 8;
pub(crate) const GROUP_WORLD_BITS: u32 = 0o077;
const JSON_SUFFIX: &str = ".json";

pub struct LocalFiles;

pub struct StagedTemp {
    pub file: File,
    pub path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StagedFile {
    pub path: PathBuf,
}

pub trait FileAccess: Send + Sync {
    fn create_parents(&self, directory: &Path) -> io::Result<()>;
    fn ensure_private_directory(&self, directory: &Path) -> io::Result<()>;
    fn directory_mode(&self, directory: &Path) -> io::Result<u32>;
    fn set_mode(&self, path: &Path, mode: u32) -> io::Result<()>;
    fn create_staged(&self, directory: &Path, suffix: Option<&str>) -> io::Result<StagedTemp>;
    fn write_staged(&self, staged: &mut StagedTemp, bytes: &[u8]) -> io::Result<()>;
    fn replace(&self, staged: &Path, target: &Path) -> io::Result<()>;
    fn remove(&self, path: &Path) -> io::Result<()>;
    fn rewrite_existing(&self, target: &Path, bytes: &[u8]) -> io::Result<()>;
}

pub struct PrivateFiles {
    access: Arc<dyn FileAccess>,
    permits: Arc<Semaphore>,
}

impl Default for PrivateFiles {
    fn default() -> Self {
        Self {
            access: Arc::new(LocalFiles),
            permits: Arc::new(Semaphore::new(MAX_CONCURRENT_FILE_OPERATIONS)),
        }
    }
}

impl From<Arc<dyn FileAccess>> for PrivateFiles {
    fn from(access: Arc<dyn FileAccess>) -> Self {
        Self {
            access,
            permits: Arc::new(Semaphore::new(MAX_CONCURRENT_FILE_OPERATIONS)),
        }
    }
}

impl PrivateFiles {
    pub async fn ensure_private_dir(&self, directory: &Path) -> Result<(), PrivateJsonError> {
        let permit = self.reserve().await?;
        let owned_directory = directory.to_owned();
        self.spawn(permit, move |files| {
            ensure_private_dir_blocking(files, &owned_directory)
        })
        .await
    }

    pub async fn stage_private_json<T>(
        &self,
        path: &Path,
        data: &T,
    ) -> Result<StagedFile, PrivateJsonError>
    where
        T: Serialize + Sync,
    {
        let bytes = serde_json::to_string_pretty(data)
            .map_err(PrivateJsonError::Encode)?
            .into_bytes();
        let permit = self.reserve().await?;
        let target = path.to_owned();
        self.spawn(permit, move |files| {
            stage_blocking(files, &target, Some(JSON_SUFFIX), &bytes)
        })
        .await
    }

    pub async fn stage_private_bytes(
        &self,
        path: &Path,
        data: &[u8],
    ) -> Result<StagedFile, PrivateJsonError> {
        let bytes = data.to_vec();
        let permit = self.reserve().await?;
        let target = path.to_owned();
        self.spawn(permit, move |files| {
            stage_blocking(files, &target, None, &bytes)
        })
        .await
    }

    pub async fn commit_staged_json(
        &self,
        staged: &StagedFile,
        path: &Path,
    ) -> Result<(), PrivateJsonError> {
        let permit = self.reserve().await?;
        let handle = staged.clone();
        let target = path.to_owned();
        self.spawn(permit, move |files| {
            commit_blocking(files, &handle, &target)
        })
        .await
    }

    pub async fn discard_staged_json(&self, staged: &StagedFile) -> Result<(), PrivateJsonError> {
        let permit = self.reserve().await?;
        let path = staged.path.clone();
        self.spawn(permit, move |files| discard_blocking(files, &path))
            .await
    }

    pub async fn overwrite_private_json<T>(
        &self,
        path: &Path,
        data: &T,
    ) -> Result<(), PrivateJsonError>
    where
        T: Serialize + Sync,
    {
        let bytes = serde_json::to_string_pretty(data)
            .map_err(PrivateJsonError::Encode)?
            .into_bytes();
        let permit = self.reserve().await?;
        let target = path.to_owned();
        self.spawn(permit, move |files| {
            overwrite_blocking(files, &target, &bytes)
        })
        .await
    }

    pub async fn write_private_json<T>(&self, path: &Path, data: &T) -> Result<(), PrivateJsonError>
    where
        T: Serialize + Sync,
    {
        let bytes = serde_json::to_string_pretty(data)
            .map_err(PrivateJsonError::Encode)?
            .into_bytes();
        let permit = self.reserve().await?;
        let target = path.to_owned();
        self.spawn(permit, move |files| {
            let staged = stage_blocking(files, &target, Some(JSON_SUFFIX), &bytes)?;
            commit_blocking(files, &staged, &target)
        })
        .await
    }

    pub async fn write_private_bytes(
        &self,
        path: &Path,
        data: &[u8],
    ) -> Result<(), PrivateJsonError> {
        let bytes = data.to_vec();
        let permit = self.reserve().await?;
        let target = path.to_owned();
        self.spawn(permit, move |files| {
            let staged = stage_blocking(files, &target, None, &bytes)?;
            commit_blocking(files, &staged, &target)
        })
        .await
    }

    async fn reserve(&self) -> Result<OwnedSemaphorePermit, PrivateJsonError> {
        Arc::clone(&self.permits)
            .acquire_owned()
            .await
            .map_err(PrivateJsonError::WorkerUnavailable)
    }

    // The owned JoinSet runs the blocking file work on tokio's bounded pool, and the permit keeps the queue bounded
    async fn spawn<T, F>(
        &self,
        permit: OwnedSemaphorePermit,
        work: F,
    ) -> Result<T, PrivateJsonError>
    where
        T: Send + 'static,
        F: FnOnce(&dyn FileAccess) -> Result<T, PrivateJsonError> + Send + 'static,
    {
        let access = Arc::clone(&self.access);
        let mut tasks = JoinSet::new();
        tasks.spawn_blocking(move || work(access.as_ref()));
        let joined = tasks
            .join_next()
            .await
            .ok_or(PrivateJsonError::WorkerStopped)?;
        drop(permit);
        joined.map_err(PrivateJsonError::Worker)?
    }
}

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::task::JoinSet;

use self::blocking::{commit_blocking, ensure_private_dir_blocking, stage_blocking};
use super::error::PrivateJsonError;

mod blocking;
mod local;

pub const PRIVATE_DIR_MODE: u32 = 0o700;
pub const PRIVATE_FILE_MODE: u32 = 0o600;
pub const MAX_CONCURRENT_FILE_OPERATIONS: usize = 8;
pub(crate) const GROUP_WORLD_BITS: u32 = 0o077;
const JSON_SUFFIX: &str = ".json";

pub type FileResult<T> = Result<T, PrivateJsonError>;

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

// The bytes a job writes and the staging suffix they are named with: JSON keeps LiteLLM's .json, raw bytes have none
struct Encoded {
    suffix: Option<&'static str>,
    bytes: Vec<u8>,
}

trait Payload {
    fn encoded(&self) -> FileResult<Encoded>;
}

struct JsonPayload<'payload, T>(&'payload T);

struct RawPayload<'payload>(&'payload [u8]);

impl<T> Payload for JsonPayload<'_, T>
where
    T: Serialize,
{
    fn encoded(&self) -> FileResult<Encoded> {
        let bytes = serde_json::to_vec_pretty(self.0).map_err(PrivateJsonError::Encode)?;
        Ok(Encoded {
            suffix: Some(JSON_SUFFIX),
            bytes,
        })
    }
}

impl Payload for RawPayload<'_> {
    fn encoded(&self) -> FileResult<Encoded> {
        Ok(Encoded {
            suffix: None,
            bytes: self.0.to_vec(),
        })
    }
}

impl PrivateFiles {
    pub async fn ensure_private_dir(&self, directory: &Path) -> FileResult<()> {
        let permit = self.reserve().await?;
        let owned_directory = directory.to_owned();
        self.spawn(permit, move |files| {
            ensure_private_dir_blocking(files, &owned_directory)
        })
        .await
    }

    pub async fn stage_private_json<T>(
        &self,
        target_path: &Path,
        data: &T,
    ) -> FileResult<StagedFile>
    where
        T: Serialize + Sync,
    {
        self.stage_encoded(target_path, JsonPayload(data).encoded()?)
            .await
    }

    pub async fn stage_private_bytes(
        &self,
        target_path: &Path,
        data: &[u8],
    ) -> FileResult<StagedFile> {
        self.stage_encoded(target_path, RawPayload(data).encoded()?)
            .await
    }

    pub async fn commit_staged_json(&self, staged: &StagedFile, path: &Path) -> FileResult<()> {
        let handle = staged.clone();
        self.run_with_target(path, move |files, target| {
            commit_blocking(files, &handle, target)
        })
        .await
    }

    // private_json.discard_staged_json unlinks missing_ok, so a staged file that is already gone is still discarded
    pub async fn discard_staged_json(&self, staged: &StagedFile) -> FileResult<()> {
        self.run_with_target(&staged.path, move |files, target| {
            match files.remove(target) {
                Ok(()) => Ok(()),
                Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(source) => Err(PrivateJsonError::Discard(target.to_owned(), source)),
            }
        })
        .await
    }

    pub async fn write_private_json<T>(&self, target_path: &Path, data: &T) -> FileResult<()>
    where
        T: Serialize + Sync,
    {
        self.write_encoded(target_path, JsonPayload(data).encoded()?)
            .await
    }

    pub async fn write_private_bytes(&self, target_path: &Path, data: &[u8]) -> FileResult<()> {
        self.write_encoded(target_path, RawPayload(data).encoded()?)
            .await
    }

    pub async fn overwrite_private_json<T>(&self, target_path: &Path, data: &T) -> FileResult<()>
    where
        T: Serialize + Sync,
    {
        self.overwrite_encoded(target_path, JsonPayload(data).encoded()?.bytes)
            .await
    }

    async fn stage_encoded(&self, path: &Path, encoded: Encoded) -> FileResult<StagedFile> {
        self.encoded_job(path, encoded.suffix, encoded.bytes, |_, staged, _| {
            Ok(staged.clone())
        })
        .await
    }

    async fn write_encoded(&self, path: &Path, encoded: Encoded) -> FileResult<()> {
        self.encoded_job(path, encoded.suffix, encoded.bytes, commit_blocking)
            .await
    }

    // Staging and the finish run in one blocking job, so a cancelled caller never leaves its staged bytes behind
    async fn encoded_job<R, F>(
        &self,
        path: &Path,
        suffix: Option<&'static str>,
        bytes: Vec<u8>,
        finish: F,
    ) -> FileResult<R>
    where
        R: Send + 'static,
        F: FnOnce(&dyn FileAccess, &StagedFile, &Path) -> FileResult<R> + Send + 'static,
    {
        self.run_with_target(path, move |files, target| {
            let staged = stage_blocking(files, target, suffix, &bytes)?;
            finish(files, &staged, target)
        })
        .await
    }

    // private_json.overwrite_private_json opens O_WRONLY | O_TRUNC; O_NOFOLLOW refuses to write through a symlink
    async fn overwrite_encoded(&self, path: &Path, encoded: Vec<u8>) -> FileResult<()> {
        self.run_with_target(path, move |files, target| {
            files
                .rewrite_existing(target, &encoded)
                .map_err(|source| PrivateJsonError::Overwrite(target.to_owned(), source))
        })
        .await
    }

    async fn reserve(&self) -> FileResult<OwnedSemaphorePermit> {
        Arc::clone(&self.permits)
            .acquire_owned()
            .await
            .map_err(PrivateJsonError::WorkerUnavailable)
    }

    async fn run_with_target<T, F>(&self, path: &Path, work: F) -> FileResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&dyn FileAccess, &Path) -> FileResult<T> + Send + 'static,
    {
        let permit = self.reserve().await?;
        let target = path.to_owned();
        self.spawn(permit, move |files| work(files, &target)).await
    }

    // The permit moves into the job because tokio cannot abort a started blocking task, so a cancelled caller keeps its slot
    async fn spawn<T, F>(&self, permit: OwnedSemaphorePermit, work: F) -> FileResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&dyn FileAccess) -> FileResult<T> + Send + 'static,
    {
        let access = Arc::clone(&self.access);
        let mut tasks = JoinSet::new();
        tasks.spawn_blocking(move || {
            let _held = permit;
            work(access.as_ref())
        });
        let joined = tasks
            .join_next()
            .await
            .ok_or(PrivateJsonError::WorkerStopped)?;
        joined.map_err(PrivateJsonError::Worker)?
    }
}

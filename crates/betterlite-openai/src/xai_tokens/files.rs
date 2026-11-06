use std::fs::Permissions;
use std::future::Future;
use std::io;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;

use nix::libc::O_NOFOLLOW;
use tokio::fs;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use super::codec::DOCUMENT_LIMIT;

pub trait FileAccess: Send + Sync {
    fn ensure_dir(&self, directory: &Path) -> impl Future<Output = io::Result<()>> + Send;
    fn read(&self, path: &Path) -> impl Future<Output = io::Result<Vec<u8>>> + Send;
    fn replace(&self, target: &Path, data: Vec<u8>) -> impl Future<Output = io::Result<()>> + Send;
}

pub struct LocalFiles;

impl FileAccess for LocalFiles {
    async fn ensure_dir(&self, directory: &Path) -> io::Result<()> {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true).mode(0o700);
        builder.create(directory).await?;
        if let Err(error) = fs::set_permissions(directory, Permissions::from_mode(0o700)).await {
            tracing::debug!(%error, "Could not chmod xAI OAuth token directory");
        }
        Ok(())
    }

    async fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        let limit = u64::try_from(DOCUMENT_LIMIT).unwrap_or(u64::MAX);
        let file = fs::File::open(path).await?;
        let mut limited = file.take(limit.saturating_add(1));
        limited.read_to_end(&mut bytes).await?;
        Ok(bytes)
    }

    async fn replace(&self, target: &Path, data: Vec<u8>) -> io::Result<()> {
        let directory = target.parent().ok_or(io::ErrorKind::NotFound)?;
        let file_name = target.file_name().and_then(|name| name.to_str());
        let name = file_name.ok_or(io::ErrorKind::NotFound)?;
        let temp = directory.join(format!(".{name}.{}.tmp", uuid::Uuid::new_v4().simple()));
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true).mode(0o600);
        options.custom_flags(O_NOFOLLOW);
        let mut file = options.open(&temp).await?;
        let written = async {
            file.write_all(&data).await?;
            file.flush().await?;
            file.sync_all().await?;
            fs::rename(&temp, target).await
        }
        .await;
        if let Err(cause) = written {
            if let Err(error) = fs::remove_file(&temp).await {
                tracing::debug!(%error, "Could not remove xAI OAuth temp file");
            }
            return Err(cause);
        }
        if let Err(error) = fs::set_permissions(target, Permissions::from_mode(0o600)).await {
            tracing::debug!(%error, "Could not chmod xAI OAuth auth file");
        }
        Ok(())
    }
}

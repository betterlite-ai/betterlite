use std::fs::{Permissions, create_dir_all};
use std::future::Future;
use std::io;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;

use tokio::fs;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

pub const FILE_LIMIT: usize = 0x0010_0000;

pub trait FileAccess {
    fn ensure_dir(&mut self, path: &Path) -> io::Result<()>;
    fn read(&mut self, path: &Path) -> impl Future<Output = io::Result<Vec<u8>>> + Send;
    fn write(&mut self, path: &Path, content: &[u8])
    -> impl Future<Output = io::Result<()>> + Send;
}

#[derive(Debug, Default)]
pub struct LocalFiles;

impl FileAccess for LocalFiles {
    fn ensure_dir(&mut self, path: &Path) -> io::Result<()> {
        if path.as_os_str().is_empty() {
            return Err(io::Error::from(io::ErrorKind::NotFound));
        }
        if path.exists() {
            Ok(())
        } else {
            create_dir_all(path)
        }
    }

    async fn read(&mut self, path: &Path) -> io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        fs::File::open(path)
            .await?
            .take(0x0010_0001)
            .read_to_end(&mut bytes)
            .await?;
        if bytes.len() > FILE_LIMIT {
            return Err(io::Error::other("the Copilot token file is too large"));
        }
        Ok(bytes)
    }

    async fn write(&mut self, path: &Path, content: &[u8]) -> io::Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .await?;
        file.set_permissions(Permissions::from_mode(0o600)).await?;
        file.write_all(content).await
    }
}

mod tests;

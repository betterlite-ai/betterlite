use std::fs::{self, File};
use std::io::{self, Read as _};
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::path::Path;

use super::codec::AuthRecord;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreEvent {
    InvalidRecord,
    WriteFailure,
}

pub trait FileAccess {
    fn ensure_dir(&mut self, path: &Path) -> io::Result<()>;
    fn read(&mut self, path: &Path) -> io::Result<Vec<u8>>;
    fn write(&mut self, path: &Path, record: &AuthRecord) -> io::Result<()>;
    fn report(&mut self, event: StoreEvent) {
        match event {
            StoreEvent::InvalidRecord => tracing::warn!("Invalid ChatGPT auth file"),
            StoreEvent::WriteFailure => tracing::error!("Failed to write ChatGPT auth file"),
        }
    }
}

pub struct LocalFiles;

impl FileAccess for LocalFiles {
    fn ensure_dir(&mut self, path: &Path) -> io::Result<()> {
        if path.as_os_str().is_empty() {
            return Err(io::Error::from(io::ErrorKind::NotFound));
        }
        if path.exists() {
            Ok(())
        } else {
            fs::create_dir_all(path)
        }
    }

    fn read(&mut self, path: &Path) -> io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        File::open(path)?
            .take(0x0010_0001)
            .read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    fn write(&mut self, path: &Path, record: &AuthRecord) -> io::Result<()> {
        let file = File::options()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        serde_json::to_writer(file, record).map_err(io::Error::other)
    }
}

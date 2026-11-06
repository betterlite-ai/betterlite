use std::fs::{self, File};
use std::io::{self, Read as _, Write as _};
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::path::Path;

pub const FILE_LIMIT: usize = 0x0010_0000;

pub trait FileAccess {
    fn ensure_dir(&mut self, path: &Path) -> io::Result<()>;
    fn read(&mut self, path: &Path) -> io::Result<Vec<u8>>;
    fn write(&mut self, path: &Path, content: &[u8]) -> io::Result<()>;
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
            fs::create_dir_all(path)
        }
    }

    fn read(&mut self, path: &Path) -> io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        File::open(path)?
            .take(0x0010_0001)
            .read_to_end(&mut bytes)?;
        if bytes.len() > FILE_LIMIT {
            return Err(io::Error::other("the Copilot token file is too large"));
        }
        Ok(bytes)
    }

    fn write(&mut self, path: &Path, content: &[u8]) -> io::Result<()> {
        let mut file = File::options()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        file.write_all(content)
    }
}

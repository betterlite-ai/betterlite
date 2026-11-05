use std::ffi::OsString;
use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, Read as _, Write as _};
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::path::Path;

use nix::libc::O_NOFOLLOW;
use uuid::Uuid;

use super::codec::DOCUMENT_LIMIT;

pub trait FileAccess: Send + Sync {
    fn ensure_dir(&self, directory: &Path) -> io::Result<()>;
    fn read(&self, path: &Path) -> io::Result<Vec<u8>>;
    fn replace(&self, directory: &Path, target: &Path, contents: &[u8]) -> io::Result<()>;
}

pub struct LocalFiles;

impl FileAccess for LocalFiles {
    fn ensure_dir(&self, directory: &Path) -> io::Result<()> {
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(directory)?;
        if let Err(error) = fs::set_permissions(directory, fs::Permissions::from_mode(0o700)) {
            tracing::debug!(%error, "Could not chmod xAI OAuth token directory");
        }
        Ok(())
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        let limit = u64::try_from(DOCUMENT_LIMIT).unwrap_or(u64::MAX);
        File::open(path)?
            .take(limit.saturating_add(1))
            .read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    fn replace(&self, directory: &Path, target: &Path, contents: &[u8]) -> io::Result<()> {
        let mut name = OsString::from(".");
        name.push(target.file_name().unwrap_or_default());
        name.push(format!(".{}.tmp", Uuid::new_v4().simple()));
        let temp = directory.join(name);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .custom_flags(O_NOFOLLOW)
            .mode(0o600)
            .open(&temp)?;
        let written = file
            .write_all(contents)
            .and_then(|()| file.flush())
            .and_then(|()| file.sync_all())
            .and_then(|()| fs::rename(&temp, target));
        if let Err(cause) = written {
            if let Err(error) = fs::remove_file(&temp) {
                tracing::debug!(%error, "Could not remove xAI OAuth temp file");
            }
            return Err(cause);
        }
        if let Err(error) = fs::set_permissions(target, fs::Permissions::from_mode(0o600)) {
            tracing::debug!(%error, "Could not chmod xAI OAuth auth file");
        }
        Ok(())
    }
}

use std::fs::{self, Permissions};
use std::io::{self, Write as _};
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::path::Path;

use nix::libc::O_NOFOLLOW;

use super::{FileAccess, LocalFiles, PRIVATE_DIR_MODE, PRIVATE_FILE_MODE, StagedTemp};

impl FileAccess for LocalFiles {
    fn create_parents(&self, directory: &Path) -> io::Result<()> {
        fs::create_dir_all(directory)
    }

    // Path.mkdir(mode=0o700, parents=True, exist_ok=True): missing parents take the umask's mode, the leaf takes 0700
    fn ensure_private_directory(&self, directory: &Path) -> io::Result<()> {
        if let Some(parent) = directory
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)?;
        }
        match fs::DirBuilder::new()
            .mode(PRIVATE_DIR_MODE)
            .create(directory)
        {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists && directory.is_dir() => {
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    fn directory_mode(&self, directory: &Path) -> io::Result<u32> {
        Ok(fs::metadata(directory)?.permissions().mode() & 0o777)
    }

    fn set_mode(&self, path: &Path, mode: u32) -> io::Result<()> {
        fs::set_permissions(path, Permissions::from_mode(mode))
    }

    fn create_staged(&self, directory: &Path, suffix: Option<&str>) -> io::Result<StagedTemp> {
        let mut builder = tempfile::Builder::new();
        builder
            .prefix(".tmp-")
            .permissions(Permissions::from_mode(PRIVATE_FILE_MODE));
        if let Some(name_suffix) = suffix {
            builder.suffix(name_suffix);
        }
        let staged = builder.tempfile_in(directory)?;
        let (file, path) = staged.keep().map_err(|error| error.error)?;
        Ok(StagedTemp { file, path })
    }

    fn write_staged(&self, staged: &mut StagedTemp, bytes: &[u8]) -> io::Result<()> {
        staged.file.write_all(bytes)?;
        staged.file.flush()?;
        staged.file.sync_all()
    }

    fn replace(&self, staged: &Path, target: &Path) -> io::Result<()> {
        fs::rename(staged, target)
    }

    fn remove(&self, path: &Path) -> io::Result<()> {
        fs::remove_file(path)
    }

    // private_json.overwrite_private_json opens O_WRONLY | O_TRUNC; O_NOFOLLOW refuses to write through a symlink
    fn rewrite_existing(&self, target: &Path, bytes: &[u8]) -> io::Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .custom_flags(O_NOFOLLOW)
            .open(target)?;
        file.write_all(bytes)?;
        file.flush()?;
        file.sync_all()
    }
}

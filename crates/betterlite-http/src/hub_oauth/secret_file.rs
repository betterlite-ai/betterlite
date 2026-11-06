use std::fs::Permissions;
use std::io;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;

use tokio::fs::{self, DirBuilder, OpenOptions};
use tokio::io::AsyncWriteExt as _;

const SECRET_FILE_MODE: u32 = 0o600;
const SECRET_DIR_MODE: u32 = 0o700;

// huggingface_hub's _write_secret: the file and its folder end up owner-only, and a chmod that fails is ignored
pub async fn write_secret(path: &Path, content: &str) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty());
    if let Some(directory) = parent {
        DirBuilder::new()
            .recursive(true)
            .mode(SECRET_DIR_MODE)
            .create(directory)
            .await?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(SECRET_FILE_MODE)
        .open(path)
        .await?;
    file.write_all(content.as_bytes()).await?;
    file.flush().await?;
    drop(file);
    if let Err(error) = fs::set_permissions(path, Permissions::from_mode(SECRET_FILE_MODE)).await {
        tracing::debug!(%error, "the secret file keeps its mode");
    }
    if let Some(directory) = parent
        && let Err(error) =
            fs::set_permissions(directory, Permissions::from_mode(SECRET_DIR_MODE)).await
    {
        tracing::debug!(%error, "the secret folder keeps its mode");
    }
    Ok(())
}

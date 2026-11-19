use std::io;
use std::path::{Component, Path, PathBuf};

use thiserror::Error;
use tokio::fs;

#[derive(Debug, Error)]
pub enum StageError {
    #[error("creating the folder {path:?} for a skill file failed")]
    CreateDir {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("writing the staged skill file {path:?} failed")]
    Write {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

pub async fn stage_file(
    root: &Path,
    raw: &str,
    content: &[u8],
) -> Result<Option<PathBuf>, StageError> {
    let Some(local) = resolved_inside(root, raw) else {
        return Ok(None);
    };
    if let Some(parent) = local.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|source| StageError::CreateDir {
                path: parent.to_path_buf(),
                source,
            })?;
    }
    fs::write(&local, content)
        .await
        .map_err(|source| StageError::Write {
            path: local.clone(),
            source,
        })?;
    Ok(Some(local))
}

// LiteLLM abspaths the joined path and refuses anything not under the staging folder
fn resolved_inside(root: &Path, raw: &str) -> Option<PathBuf> {
    let joined = root.join(raw);
    let mut resolved = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::ParentDir => {
                resolved.pop();
            }
            Component::Normal(_)
            | Component::Prefix(_)
            | Component::RootDir
            | Component::CurDir => {
                resolved.push(component.as_os_str());
            }
        }
    }
    let relative = resolved.strip_prefix(root).ok()?;
    if relative.as_os_str().is_empty() {
        return None;
    }
    Some(resolved)
}

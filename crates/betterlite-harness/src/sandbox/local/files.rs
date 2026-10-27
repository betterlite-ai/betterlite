use std::env;
use std::fs::{self, File};
use std::io::{self, Read as _};
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Component, Path, PathBuf};

use super::LocalSandbox;
use super::error::SandboxError;
use crate::errors::{CappedOperation, HarnessError};

pub(super) fn path_string(path: &Path) -> Result<String, HarnessError> {
    let text = path
        .to_str()
        .ok_or_else(|| SandboxError::PathEncoding(path.to_path_buf()))?;
    Ok(text.to_owned())
}

pub(super) fn real_path(path: &Path, depth: u8) -> io::Result<PathBuf> {
    if depth >= 40 {
        return Err(io::Error::other("too many symbolic links"));
    }
    let mut resolved = PathBuf::new();
    for part in path.components() {
        match part {
            Component::ParentDir => {
                resolved.pop();
            }
            Component::CurDir => {}
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                resolved.push(part);
                match fs::symlink_metadata(&resolved) {
                    Ok(metadata) if metadata.is_symlink() => {
                        let target = fs::read_link(&resolved)?;
                        resolved.pop();
                        resolved = real_path(&resolved.join(target), depth.saturating_add(1))?;
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
            }
        }
    }
    Ok(resolved)
}

pub(super) fn read_file(path: &Path, max: usize) -> Result<Vec<u8>, HarnessError> {
    let file = File::open(path).map_err(|source| HarnessError::Sandbox { source })?;
    let mut data = Vec::new();
    file.take(u64::try_from(max).unwrap_or(u64::MAX).saturating_add(1))
        .read_to_end(&mut data)
        .map_err(|source| HarnessError::Sandbox { source })?;
    if data.len() > max {
        return Err(HarnessError::ContentLimitExceeded {
            operation: CappedOperation::Read,
            limit: max,
        });
    }
    Ok(data)
}

pub(super) fn write_file(path: &Path, data: &[u8]) -> Result<(), HarnessError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| HarnessError::Sandbox { source })?;
    }
    fs::write(path, data).map_err(|source| HarnessError::Sandbox { source })
}

pub(super) fn find_binary(binary: &str, path: &str, cwd: &Path) -> Option<PathBuf> {
    let candidates = if binary.contains('/') {
        vec![cwd.join(binary)]
    } else {
        env::split_paths(path)
            .map(|directory| cwd.join(directory).join(binary))
            .collect()
    };
    candidates.into_iter().find(|candidate| {
        candidate
            .metadata()
            .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
    })
}

pub(super) fn decode_utf8(bytes: &[u8]) -> String {
    let mut text = String::new();
    for chunk in bytes.utf8_chunks() {
        text.push_str(chunk.valid());
        if !chunk.invalid().is_empty() {
            text.push('\u{fffd}');
        }
    }
    text
}

pub(super) async fn operation<Job, Output>(
    sandbox: &LocalSandbox,
    path: &str,
    job: Job,
) -> Result<Output, HarnessError>
where
    Job: FnOnce(&Path) -> Result<Output, HarnessError> + Send + 'static,
    Output: Send + 'static,
{
    let name = path.to_owned();
    sandbox
        .access(move |state, root| {
            let resolved = state.resolve(root, &name)?;
            job(&resolved)
        })
        .await
}

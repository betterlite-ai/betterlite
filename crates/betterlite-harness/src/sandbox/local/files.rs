use std::fs::{self, File};
use std::io::{self, Read as _};
use std::path::{Path, PathBuf};

use soft_canonicalize::soft_canonicalize;

use super::LocalSandbox;
use super::error::SandboxError;
use crate::errors::{CappedOperation, HarnessError};

pub(super) fn path_string(path: &Path) -> Result<String, HarnessError> {
    let text = path
        .to_str()
        .ok_or_else(|| SandboxError::PathEncoding(path.to_path_buf()))?;
    Ok(text.to_owned())
}

pub(super) fn real_path(path: &Path) -> io::Result<PathBuf> {
    soft_canonicalize(path)
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

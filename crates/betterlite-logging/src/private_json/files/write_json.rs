use std::path::Path;

use serde::Serialize;

use super::blocking::{commit_blocking, stage_blocking};
use super::{FileResult, JSON_SUFFIX, PrivateFiles};
use crate::private_json::error::PrivateJsonError;

// Staging and committing happen in one blocking job, so a cancelled caller never leaves its staged bytes behind
pub async fn write_private_json<T>(files: &PrivateFiles, path: &Path, data: &T) -> FileResult<()>
where
    T: Serialize + Sync,
{
    let bytes = serde_json::to_vec_pretty(data).map_err(PrivateJsonError::Encode)?;
    files
        .run_with_target(path, move |access, target| {
            let staged = stage_blocking(access, target, Some(JSON_SUFFIX), &bytes)?;
            commit_blocking(access, &staged, target)
        })
        .await
}

use std::path::Path;

use serde::Serialize;

use super::blocking::stage_blocking;
use super::{FileResult, JSON_SUFFIX, PrivateFiles, StagedFile};
use crate::private_json::error::PrivateJsonError;

pub async fn stage_private_json<T>(
    files: &PrivateFiles,
    path: &Path,
    data: &T,
) -> FileResult<StagedFile>
where
    T: Serialize + Sync,
{
    let bytes = serde_json::to_vec_pretty(data).map_err(PrivateJsonError::Encode)?;
    files
        .run_with_target(path, move |access, target| {
            stage_blocking(access, target, Some(JSON_SUFFIX), &bytes)
        })
        .await
}

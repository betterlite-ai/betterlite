use std::path::Path;

use serde::Serialize;

use super::{FileResult, PrivateFiles};
use crate::private_json::error::PrivateJsonError;

// private_json.overwrite_private_json opens O_WRONLY | O_TRUNC; O_NOFOLLOW refuses to write through a symlink
pub async fn overwrite_private_json<T>(
    files: &PrivateFiles,
    path: &Path,
    data: &T,
) -> FileResult<()>
where
    T: Serialize + Sync,
{
    let document = serde_json::to_vec_pretty(data).map_err(PrivateJsonError::Encode)?;
    files
        .run_with_target(path, move |access, destination| {
            access
                .rewrite_existing(destination, &document)
                .map_err(|source| PrivateJsonError::Overwrite(destination.to_owned(), source))
        })
        .await
}

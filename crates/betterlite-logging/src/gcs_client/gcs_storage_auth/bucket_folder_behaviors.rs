use betterlite_context::cloud_storage::{CloudStorageError, split_bucket_name};

pub(super) fn handle_folders_in_bucket_name(
    bucket_name: &str,
    object_name: &str,
) -> Result<(String, String), CloudStorageError> {
    let (bucket, prefix) = split_bucket_name(bucket_name)?;
    if prefix.is_empty() {
        return Ok((bucket, object_name.to_owned()));
    }
    Ok((bucket, format!("{prefix}/{object_name}")))
}

mod tests;

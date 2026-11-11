#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error(
    "{failed} of {total} S3 uploads failed; transient failures kept in queue for the next flush"
)]
pub struct S3BatchUploadError {
    pub failed: usize,
    pub total: usize,
}

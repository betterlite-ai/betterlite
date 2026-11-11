use betterlite_models::decisions::Json;

use crate::s3_transport::batch_element::S3BatchLoggingElement;

// one queue element is any JSON object, so audit records share the model-event queue
pub type Element = S3BatchLoggingElement<Json>;

pub mod flush;
pub mod logger;
pub mod ndjson_batch_file;
pub mod queue;
pub mod upload_bounded;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error(
    "{failed} of {total} S3 uploads failed; transient failures kept in queue for the next flush"
)]
pub struct S3BatchUploadError {
    pub failed: usize,
    pub total: usize,
}

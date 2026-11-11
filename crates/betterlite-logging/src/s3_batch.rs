use betterlite_models::decisions::Json;

use crate::s3_transport::batch_element::S3BatchLoggingElement;

// one queue element is any JSON object, so audit records share the model-event queue
pub type Element = S3BatchLoggingElement<Json>;

pub mod credentials;
pub mod error;
pub mod flush;
pub mod logger;
pub mod ndjson_batch_file;
pub mod queue;
pub mod upload_bounded;

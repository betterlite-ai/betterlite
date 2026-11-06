use std::error::Error;

use crate::finops::metering::lago::payload::LagoPayloadError;

#[derive(Debug, thiserror::Error)]
pub enum LagoError<SenderError>
where
    SenderError: Error + 'static,
{
    #[error("the Lago event could not be encoded as JSON: {0}")]
    Encode(#[from] serde_json::Error),
    #[error(transparent)]
    Payload(#[from] LagoPayloadError),
    #[error("the Lago event could not be sent: {0}")]
    Send(#[source] SenderError),
    #[error("Lago answered the event with status {status}")]
    Status { status: u16 },
    #[error("the Lago API base URL is not valid: {0}")]
    Url(#[source] url::ParseError),
}

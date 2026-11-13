use std::error::Error;

use crate::finops::metering::openmeter::payload::OpenMeterPayloadError;

#[derive(Debug, thiserror::Error)]
pub enum OpenMeterError<SenderError>
where
    SenderError: Error + 'static,
{
    #[error("the OpenMeter event could not be encoded as JSON: {0}")]
    Encode(#[from] serde_json::Error),
    #[error(transparent)]
    Payload(#[from] OpenMeterPayloadError),
    #[error("the OpenMeter event could not be sent: {0}")]
    Send(#[source] SenderError),
    #[error("OpenMeter logging error: {body}")]
    Status { status: u16, body: String },
    #[error("the OpenMeter endpoint URL is not valid: {0}")]
    Url(#[source] url::ParseError),
}

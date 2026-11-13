use std::error::Error;

use crate::finops::vantage::columns::ColumnError;

#[derive(Debug, thiserror::Error)]
pub enum DeliveryError<SendError>
where
    SendError: Error + 'static,
{
    #[error("the CSV columns could not be filtered")]
    Columns(#[source] ColumnError),
    #[error("Vantage answered with HTTP status {status}")]
    Status { status: u16 },
    #[error("the upload could not be sent: {0}")]
    Send(#[source] SendError),
}

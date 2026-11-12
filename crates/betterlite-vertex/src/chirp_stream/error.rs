use google_cloud_gax::client_builder::Error as BuilderError;
use google_cloud_gax::error::Error as ClientError;
use google_cloud_gax::streaming::SendError;

use super::target::AccessTokenError;

#[derive(Debug, thiserror::Error)]
pub enum ChirpStreamError {
    #[error("the Speech-to-Text client is closed")]
    ClientClosed,
    #[error("the Speech-to-Text client could not be opened")]
    Open(#[source] BuilderError),
    #[error("the Speech-to-Text request could not be sent")]
    Send(#[source] SendError),
    #[error("the Speech-to-Text response stream failed")]
    Response(#[source] ClientError),
    #[error("the Speech-to-Text access token could not be resolved")]
    Token(#[source] AccessTokenError),
    #[error("the Speech-to-Text stream queue is closed")]
    QueueClosed,
}

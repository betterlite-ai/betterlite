use std::io;

use betterlite_http::proxy::transport_error::TransportError;

use super::codec::DecodeError;

#[derive(Debug, thiserror::Error)]
pub enum XaiOauthError {
    #[error("xAI OAuth login required")]
    LoginRequired,
    #[error("xAI OAuth refresh token missing")]
    RefreshTokenMissing,
    #[error("xAI OAuth {stage} request could not be sent")]
    Transport {
        stage: &'static str,
        #[source]
        source: TransportError,
    },
    #[error("xAI OAuth {stage} request failed with status {status}")]
    Status { stage: &'static str, status: u16 },
    #[error("xAI OAuth {stage} response was not a valid JSON object")]
    Response {
        stage: &'static str,
        #[source]
        source: DecodeError,
    },
    #[error("xAI OAuth discovery missing endpoints")]
    MissingEndpoints,
    #[error("xAI OAuth returned an endpoint outside https x.ai")]
    UnexpectedEndpoint,
    #[error("xAI OAuth token response missing access_token")]
    MissingAccessToken,
    #[error("xAI OAuth token response missing refresh_token")]
    MissingRefreshToken,
    #[error("cannot write the xAI OAuth auth file")]
    Write(#[source] io::Error),
}

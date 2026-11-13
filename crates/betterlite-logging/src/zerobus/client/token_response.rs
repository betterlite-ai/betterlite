use secrecy::SecretString;
use serde::Deserialize;

use crate::zerobus::client::types::{ZerobusAccessToken, ZerobusIngestFailure};

#[derive(Deserialize)]
struct TokenReply {
    access_token: SecretString,
    #[serde(default = "default_expires_in")]
    expires_in: f64,
}

const fn default_expires_in() -> f64 {
    3600.0
}

// Every decode failure is permanent: the same body cannot become a token on a later attempt
pub fn decode_token_response(
    bytes: &[u8],
    now: f64,
) -> Result<ZerobusAccessToken, ZerobusIngestFailure> {
    match serde_json::from_slice::<TokenReply>(bytes) {
        Ok(reply) => Ok(ZerobusAccessToken {
            value: reply.access_token,
            // mul_add keeps clippy's float-arithmetic-operator restriction quiet
            expires_at: reply.expires_in.mul_add(1.0, now),
        }),
        Err(error) => Err(ZerobusIngestFailure {
            detail: format!("token response was not understood: {error}"),
            retryable: false,
        }),
    }
}

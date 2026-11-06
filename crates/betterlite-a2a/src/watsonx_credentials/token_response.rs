use num_traits::ToPrimitive as _;
use secrecy::SecretString;
use serde::Deserialize;
use thiserror::Error;

mod tests;

const DEFAULT_TOKEN_LIFETIME_SECONDS: i64 = 3600;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TokenFlow {
    IbmCloud,
    Cp4d,
}

#[derive(Debug)]
pub struct TokenGrant {
    pub token: SecretString,
    pub lifetime_seconds: i64,
}

#[derive(Debug, Error)]
#[error("token response is not the expected JSON")]
pub struct TokenResponseError {
    #[source]
    source: serde_json::Error,
}

#[derive(Deserialize)]
struct IbmCloudBody {
    access_token: String,
    #[serde(default)]
    expires_in: Option<i64>,
}

#[derive(Deserialize)]
struct Cp4dBody {
    token: String,
    #[serde(default)]
    expiration: Option<f64>,
}

// CP4D sends an absolute Unix time in seconds, not a duration
fn remaining_seconds(expires_at: f64, now_unix_seconds: i64) -> i64 {
    let epoch = match expires_at.to_i64() {
        Some(epoch) => epoch,
        None if expires_at.is_sign_positive() => i64::MAX,
        None => i64::MIN,
    };
    epoch.saturating_sub(now_unix_seconds).max(0)
}

pub fn decode_wxo_token_response(
    body: &[u8],
    flow: TokenFlow,
    now_unix_seconds: i64,
) -> Result<TokenGrant, TokenResponseError> {
    let (token, lifetime_seconds) = match flow {
        TokenFlow::IbmCloud => {
            let parsed = serde_json::from_slice::<IbmCloudBody>(body)
                .map_err(|source| TokenResponseError { source })?;
            (
                parsed.access_token,
                parsed.expires_in.unwrap_or(DEFAULT_TOKEN_LIFETIME_SECONDS),
            )
        }
        TokenFlow::Cp4d => {
            let parsed = serde_json::from_slice::<Cp4dBody>(body)
                .map_err(|source| TokenResponseError { source })?;
            let lifetime = parsed
                .expiration
                .map_or(DEFAULT_TOKEN_LIFETIME_SECONDS, |at| {
                    remaining_seconds(at, now_unix_seconds)
                });
            (parsed.token, lifetime)
        }
    };
    Ok(TokenGrant {
        token: SecretString::from(token),
        lifetime_seconds,
    })
}

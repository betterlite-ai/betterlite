use secrecy::SecretString;
use serde::Deserialize;

use super::error::SecretLookupError;

#[derive(Deserialize)]
struct GithubTokenBody {
    #[serde(default)]
    value: Option<String>,
}

pub fn decode_github_token(body: &[u8]) -> Result<Option<SecretString>, SecretLookupError> {
    if body.trim_ascii_start().first() != Some(&b'{') {
        return Err(SecretLookupError::GithubTokenNotObject);
    }
    let document: GithubTokenBody = serde_json::from_slice(body)
        .map_err(|source| SecretLookupError::GithubTokenDocument { source })?;
    Ok(document.value.map(SecretString::from))
}

use secrecy::SecretString;
use serde::Deserialize;

use super::error::SecretLookupError;

#[derive(Deserialize)]
struct GithubTokenDocument {
    #[serde(default)]
    value: Option<String>,
}

pub fn decode_github_token(body: &[u8]) -> Result<Option<SecretString>, SecretLookupError> {
    let document: GithubTokenDocument = serde_json::from_slice(body)
        .map_err(|source| SecretLookupError::GithubTokenDocument { source })?;
    Ok(document.value.map(SecretString::from))
}

use secrecy::{ExposeSecret as _, SecretString};
use sha2::{Digest as _, Sha256};

use crate::azure_urls::AuthParams;

use super::projection::{CallableProvider, ClientIdentity, LiteLlmParams};

const NONE: &str = "None";

// get_azure_openai_client hashes the token, the provider object identity and both passwords into its cache key
#[must_use]
pub fn client_identity(
    api_key: Option<&SecretString>,
    is_async: bool,
    params: &LiteLlmParams,
    provider: Option<&CallableProvider>,
) -> ClientIdentity {
    ClientIdentity {
        is_async,
        hashed_api_key: api_key.map(|key| sha256_hex(key.expose_secret())),
        hashed_ad_token: params
            .azure_ad_token
            .as_ref()
            .map(|token| sha256_hex(token.expose_secret())),
        provider_identity: provider_identity(&params.auth, provider.map(|value| value.identity)),
    }
}

fn provider_identity(params: &AuthParams, provider_id: Option<u64>) -> String {
    let provider_text = provider_id.map_or_else(|| NONE.to_owned(), |value| value.to_string());
    format!(
        "provider_id={provider_text}|tenant_id={}|client_id={}|client_secret={}|azure_username={}|azure_password={}|azure_scope={}",
        params.tenant_id.as_deref().unwrap_or(NONE),
        params.client_id.as_deref().unwrap_or(NONE),
        hashed_secret(params.client_secret.as_ref()),
        params.azure_username.as_deref().unwrap_or(NONE),
        hashed_secret(params.azure_password.as_ref()),
        params.azure_scope.as_deref().unwrap_or(NONE),
    )
}

fn hashed_secret(secret: Option<&SecretString>) -> String {
    let Some(value) = secret else {
        return NONE.to_owned();
    };
    sha256_hex(value.expose_secret())
}

fn sha256_hex(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    hex::encode(hasher.finalize())
}

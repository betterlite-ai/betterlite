use crate::api::vector_store_records::records::vector::VectorStoreIndexEndpoints;
use crate::credentials::{
    ExplicitCredentials, LiteLlmGlobals, get_openai_credentials, resolve_openai_api_base,
};
use betterlite_config::sections::openai_credentials::Settings as OpenAiCredentialsSettings;
use betterlite_http::url_path::encode_segment;
use error::VectorStoreError;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

pub mod error;
pub mod files;
pub mod reply;
pub mod store;
mod tests;

pub type Headers = BTreeMap<String, SecretString>;

const ASSISTANTS_HEADER_KEY: &str = "OpenAI-Beta";
const ASSISTANTS_HEADER_VALUE: &str = "assistants=v2";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Request {
    pub url: String,
    pub body: Value,
}

pub fn get_auth_credentials(api_key: Option<&SecretString>) -> Result<Headers, VectorStoreError> {
    let key = api_key.ok_or(VectorStoreError::MissingApiKey)?;
    let bearer = format!("Bearer {}", key.expose_secret());
    Ok(BTreeMap::from([(
        "Authorization".to_owned(),
        SecretString::from(bearer),
    )]))
}

#[must_use]
pub fn validate_environment(
    mut headers: Headers,
    explicit_key: Option<SecretString>,
    globals: LiteLlmGlobals,
    env_api_key: Option<SecretString>,
) -> Headers {
    let explicit = ExplicitCredentials {
        api_base: None,
        api_key: explicit_key,
        organization: None,
    };
    let settings = OpenAiCredentialsSettings::default();
    let resolved = get_openai_credentials(explicit, globals, &settings, env_api_key);
    let key = resolved
        .api_key
        .as_ref()
        .map_or("None", |key| key.expose_secret());
    headers.insert(
        "Authorization".to_owned(),
        SecretString::from(format!("Bearer {key}")),
    );
    headers.insert(
        "Content-Type".to_owned(),
        SecretString::from("application/json"),
    );
    headers
        .entry(ASSISTANTS_HEADER_KEY.to_owned())
        .or_insert_with(|| SecretString::from(ASSISTANTS_HEADER_VALUE));
    headers
}

pub fn complete_url(
    api_base: Option<&str>,
    litellm_api_base: Option<&str>,
    settings: &OpenAiCredentialsSettings,
    vector_store_id: Option<&str>,
) -> Result<String, VectorStoreError> {
    let resolved = resolve_openai_api_base(api_base, litellm_api_base, settings);
    let base = resolved.trim_end_matches('/');
    let Some(id) = vector_store_id else {
        return Ok(format!("{base}/vector_stores"));
    };
    let encoded = encode_segment(id, "vector_store_id")?;
    Ok(format!("{base}/vector_stores/{encoded}/files"))
}

fn endpoints(read: &[(&str, &str)], write: &[(&str, &str)]) -> VectorStoreIndexEndpoints {
    let owned = |routes: &[(&str, &str)]| {
        routes
            .iter()
            .map(|(method, path)| ((*method).to_owned(), (*path).to_owned()))
            .collect()
    };
    VectorStoreIndexEndpoints {
        read: Some(owned(read)),
        write: Some(owned(write)),
        extra: BTreeMap::new(),
    }
}

#[must_use]
pub fn store_endpoints() -> VectorStoreIndexEndpoints {
    endpoints(
        &[("GET", "/vector_stores/{index_name}/search")],
        &[("POST", "/vector_stores")],
    )
}

#[must_use]
pub fn file_endpoints() -> VectorStoreIndexEndpoints {
    endpoints(
        &[
            ("GET", "/vector_stores/{vector_store_id}/files"),
            ("GET", "/vector_stores/{vector_store_id}/files/{file_id}"),
            (
                "GET",
                "/vector_stores/{vector_store_id}/files/{file_id}/content",
            ),
        ],
        &[
            ("POST", "/vector_stores/{vector_store_id}/files"),
            ("POST", "/vector_stores/{vector_store_id}/files/{file_id}"),
            ("DELETE", "/vector_stores/{vector_store_id}/files/{file_id}"),
        ],
    )
}

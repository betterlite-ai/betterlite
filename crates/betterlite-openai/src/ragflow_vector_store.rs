use std::collections::BTreeMap;

use secrecy::{ExposeSecret as _, SecretString};

use crate::api::vector_store_records::records::vector::{
    VectorStoreIndexEndpoints, VectorStoreSearchResponse,
};
use crate::vector_stores::{Headers, Request};
use error::{RagflowError, SearchTransform};

pub mod create;
pub mod decode;
pub mod error;
mod tests;

const DEFAULT_API_BASE: &str = "http://localhost:9380";
const DATASETS_PATH: &str = "/api/v1/datasets";

#[derive(Debug, Clone, Default)]
pub struct RagflowEnv {
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
}

pub fn get_auth_credentials(
    litellm_api_key: Option<&SecretString>,
    env: &RagflowEnv,
) -> Result<Headers, RagflowError> {
    let api_key = litellm_api_key
        .or_else(|| {
            env.api_key
                .as_ref()
                .filter(|key| !key.expose_secret().is_empty())
        })
        .ok_or(RagflowError::MissingAuthKey)?;
    Ok(BTreeMap::from([(
        "Authorization".to_owned(),
        SecretString::from(format!("Bearer {}", api_key.expose_secret())),
    )]))
}

pub fn validate_environment(
    mut headers: Headers,
    litellm_api_key: Option<&SecretString>,
    env: &RagflowEnv,
) -> Result<Headers, RagflowError> {
    let explicit = litellm_api_key.filter(|key| !key.expose_secret().is_empty());
    let from_env = env
        .api_key
        .as_ref()
        .filter(|key| !key.expose_secret().is_empty());
    let api_key = explicit
        .or(from_env)
        .ok_or(RagflowError::MissingEnvironmentKey)?;
    headers.insert(
        "Authorization".to_owned(),
        SecretString::from(format!("Bearer {}", api_key.expose_secret())),
    );
    headers.insert(
        "Content-Type".to_owned(),
        SecretString::from("application/json"),
    );
    Ok(headers)
}

#[must_use]
pub fn get_complete_url(
    api_base: Option<&str>,
    litellm_api_base: Option<&str>,
    env: &RagflowEnv,
) -> String {
    let base = api_base
        .filter(|base| !base.is_empty())
        .or_else(|| litellm_api_base.filter(|base| !base.is_empty()))
        .or_else(|| env.api_base.as_deref().filter(|base| !base.is_empty()))
        .unwrap_or(DEFAULT_API_BASE);
    format!("{}{DATASETS_PATH}", base.trim_end_matches('/'))
}

#[must_use]
pub const fn endpoints() -> VectorStoreIndexEndpoints {
    VectorStoreIndexEndpoints {
        read: Some(Vec::new()),
        write: Some(Vec::new()),
        extra: BTreeMap::new(),
    }
}

pub const fn search_request() -> Result<Request, RagflowError> {
    Err(RagflowError::SearchUnsupported {
        transform: SearchTransform::Request,
    })
}

pub const fn search_response() -> Result<VectorStoreSearchResponse, RagflowError> {
    Err(RagflowError::SearchUnsupported {
        transform: SearchTransform::Response,
    })
}

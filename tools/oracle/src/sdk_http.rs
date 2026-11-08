use std::collections::BTreeMap;
use std::str;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use betterlite_provider::api::vector_store_contracts::config::Params;

use crate::case::{Compare, UpstreamAnswer};
use error::SdkHttpError;
use result::SdkResult;

pub(crate) mod capture;
mod child;
pub(crate) mod embedding_executor;
pub mod error;
pub(crate) mod native;
pub(crate) mod python;
pub mod result;
mod tests;
pub(crate) mod vector_query_embedding;

#[derive(Debug, Clone)]
pub struct OracleLimits {
    pub max_case_bytes: usize,
    pub max_result_bytes: usize,
    pub max_body_bytes: usize,
    pub max_requests: usize,
    pub max_execution: Duration,
}

impl Default for OracleLimits {
    fn default() -> Self {
        Self {
            max_case_bytes: 0x0010_0000,
            max_result_bytes: 0x0010_0000,
            max_body_bytes: 0x0100_0000,
            max_requests: 128,
            max_execution: Duration::from_secs(120),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SdkHttpCase {
    kind: String,
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) env: BTreeMap<String, String>,
    pub(crate) call: SdkHttpCall,
    pub(crate) upstream: Vec<UpstreamAnswer>,
    #[serde(default)]
    pub(crate) compare: Compare,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SdkHttpCall {
    OpenaiChat(OpenaiChat),
    VectorStoreEmbeddingExecutor(VectorStoreEmbeddingCall),
    VectorQueryEmbedding(VectorQueryEmbeddingCall),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenaiChat {
    pub(crate) model: String,
    pub(crate) messages: Vec<ChatMessage>,
    pub(crate) api_key: String,
    pub(crate) timeout_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VectorStoreEmbeddingCall {
    pub(crate) model: String,
    pub(crate) query: String,
    #[serde(default)]
    pub(crate) configuration: Params,
    #[serde(default)]
    pub(crate) metadata: Params,
    #[serde(default)]
    pub(crate) router: Option<Vec<RouterDeploymentSpec>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VectorQueryEmbeddingCall {
    pub(crate) query_text: String,
    #[serde(default)]
    pub(crate) litellm_params: Params,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouterDeploymentSpec {
    pub(crate) model_name: String,
    pub(crate) litellm_params: RouterDeploymentParams,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouterDeploymentParams {
    pub(crate) model: String,
    pub(crate) api_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatMessage {
    pub(crate) role: String,
    pub(crate) content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Side {
    Python,
    Native,
}

// the left side is always the pinned LiteLLM SDK; --self-check compares it with a second fresh process
pub(crate) const fn sides(self_check: bool) -> [Side; 2] {
    if self_check {
        [Side::Python, Side::Python]
    } else {
        [Side::Python, Side::Native]
    }
}

pub fn decode_sdk_http_case(
    bytes: &[u8],
    limits: &OracleLimits,
) -> Result<SdkHttpCase, SdkHttpError> {
    if bytes.len() > limits.max_case_bytes {
        return Err(SdkHttpError::CaseTooLarge {
            bytes: bytes.len(),
            limit: limits.max_case_bytes,
        });
    }
    let text = str::from_utf8(bytes).map_err(SdkHttpError::Utf8)?;
    let case: SdkHttpCase =
        serde_saphyr::from_str(text).map_err(|source| SdkHttpError::CaseParse(Box::new(source)))?;
    validate(&case, limits)?;
    Ok(case)
}

pub fn decode_sdk_http_result(
    bytes: &[u8],
    limits: &OracleLimits,
) -> Result<SdkResult, SdkHttpError> {
    if bytes.len() > limits.max_result_bytes {
        return Err(SdkHttpError::ResultTooLarge {
            bytes: bytes.len(),
            limit: limits.max_result_bytes,
        });
    }
    serde_json::from_slice(bytes).map_err(SdkHttpError::ResultParse)
}

// the harness owns the child's proxy routing, interpreter startup and loader paths, so a case never sets them
fn reserved_environment(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    matches!(
        upper.as_str(),
        "ALL_PROXY"
            | "HOME"
            | "HTTP_PROXY"
            | "HTTPS_PROXY"
            | "LITELLM_LOCAL_MODEL_COST_MAP"
            | "NO_PROXY"
            | "PATH"
    ) || upper.starts_with("PYTHON")
        || upper.starts_with("UV_")
}

fn validate_call(call: &SdkHttpCall, limits: &OracleLimits) -> Result<(), SdkHttpError> {
    match call {
        SdkHttpCall::OpenaiChat(chat) => {
            if chat.timeout_seconds == 0 || chat.timeout_seconds > limits.max_execution.as_secs() {
                return Err(SdkHttpError::Invalid(
                    "timeout_seconds must be between 1 and the execution limit".to_owned(),
                ));
            }
        }
        SdkHttpCall::VectorStoreEmbeddingExecutor(embedding) => {
            for deployment in embedding.router.iter().flatten() {
                if !deployment.litellm_params.model.starts_with("openai/") {
                    return Err(SdkHttpError::Invalid(
                        "every router deployment must serve an openai/ model".to_owned(),
                    ));
                }
            }
        }
        SdkHttpCall::VectorQueryEmbedding(_) => {}
    }
    Ok(())
}

fn validate(case: &SdkHttpCase, limits: &OracleLimits) -> Result<(), SdkHttpError> {
    let invalid = |message: &str| Err(SdkHttpError::Invalid(message.to_owned()));
    if case.kind != "sdk_http" || case.name.is_empty() {
        return invalid("the case needs kind sdk_http and a name");
    }
    if let Some(name) = case.env.keys().find(|name| reserved_environment(name)) {
        return invalid(&format!("the case env may not set {name}"));
    }
    validate_call(&case.call, limits)?;
    if case.upstream.len() > limits.max_requests {
        return invalid("the case scripts more provider answers than the request limit");
    }
    for answer in &case.upstream {
        if answer.body.is_some() == answer.events.is_some() {
            return invalid("an upstream answer needs exactly one of body or events");
        }
        if !(100..=599).contains(&answer.status) {
            return invalid("an upstream answer has an invalid status");
        }
    }
    if !case.compare.headers.is_empty() || !case.compare.headers_present.is_empty() {
        return invalid("compare.headers and compare.headers_present describe gateway responses");
    }
    for ignored in &case.compare.ignore {
        if !ignored.pointer.is_empty() && !ignored.pointer.starts_with('/') {
            return invalid("compare.ignore pointers must be JSON pointers");
        }
        if ignored.why.is_empty() {
            return invalid("compare.ignore entries need a why");
        }
    }
    Ok(())
}

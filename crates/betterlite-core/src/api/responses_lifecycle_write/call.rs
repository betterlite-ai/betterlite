use std::borrow::Cow;
use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;

use crate::api::response_id_codecs::response::decode_response_id;
use crate::api::responses_write_http::service::ClientKind;

// Which of the pair a call is; the two entries read different async kwargs flags and word their refusal differently
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleOperation {
    Delete,
    Cancel,
}

impl LifecycleOperation {
    // LiteLLM's refusal ValueError spells the operation in upper case
    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Delete => "DELETE",
            Self::Cancel => "CANCEL",
        }
    }
}

// The kwargs GenericLiteLLMParams and the pre-call logging update read
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LifecycleWriteParams {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub ssl_verify: Option<bool>,
    pub litellm_call_id: Option<String>,
    pub metadata: Option<Json>,
    pub litellm_metadata: Option<Json>,
}

// One delete_responses or cancel_responses call, as its kwargs reach the SDK entry point
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResponsesLifecycleWriteCall {
    pub operation: LifecycleOperation,
    pub response_id: String,
    #[serde(default)]
    pub custom_llm_provider: Option<String>,
    #[serde(default)]
    pub extra_headers: Option<BTreeMap<String, String>>,
    // LiteLLM takes extra_query and never gives it to the handler
    #[serde(default)]
    pub extra_query: Option<IndexMap<String, Json>>,
    #[serde(default)]
    pub extra_body: Option<IndexMap<String, Json>>,
    #[serde(default)]
    pub timeout: Option<f64>,
    #[serde(default)]
    pub params: LifecycleWriteParams,
    // each async entry sets its own flag; the sync entry pops its own with the identity check against true
    #[serde(default)]
    pub adelete_responses: Option<Json>,
    #[serde(default)]
    pub acancel_responses: Option<Json>,
}

// The id and provider a B903 decode yields; a nonempty decoded value overrides the argument, as LiteLLM's `or` does
pub(super) struct Resolved<'call> {
    pub(super) response_id: Cow<'call, str>,
    pub(super) provider: Option<String>,
}

// ProviderConfigManager._get_python_responses_api_config(model=None) at the provider whose config core builds
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponsesLifecycleProvider {
    OpenAi,
}

impl ResponsesLifecycleProvider {
    pub(super) const fn resolve(name: &str) -> Option<Self> {
        match name {
            "openai" => Some(Self::OpenAi),
            _ => None,
        }
    }
}

impl ResponsesLifecycleWriteCall {
    // the sync entry picks its handler from its own flag: only the boolean true satisfies `is True`
    pub(super) const fn client_kind(&self) -> ClientKind {
        let flag = match self.operation {
            LifecycleOperation::Delete => &self.adelete_responses,
            LifecycleOperation::Cancel => &self.acancel_responses,
        };
        match flag {
            Some(Json::Bool(true)) => ClientKind::Asynchronous,
            Some(
                Json::Bool(false)
                | Json::Null
                | Json::Number(_)
                | Json::Text(_)
                | Json::List(_)
                | Json::Object(_),
            )
            | None => ClientKind::Sync,
        }
    }

    // the async entry decodes first, then hands the sync entry that id and provider with its own flag set
    pub(super) fn decoded(&self, resolved: &Resolved<'_>) -> Self {
        let mut call = self.clone();
        call.response_id = resolved.response_id.clone().into_owned();
        call.custom_llm_provider.clone_from(&resolved.provider);
        match call.operation {
            LifecycleOperation::Delete => call.adelete_responses = Some(Json::Bool(true)),
            LifecycleOperation::Cancel => call.acancel_responses = Some(Json::Bool(true)),
        }
        call
    }
}

pub(super) fn resolve(call: &ResponsesLifecycleWriteCall) -> Resolved<'_> {
    let decoded = decode_response_id(&call.response_id);
    let response_id = match decoded.response_id.filter(|id| !id.is_empty()) {
        Some(id) => Cow::Owned(id),
        None => Cow::Borrowed(call.response_id.as_str()),
    };
    Resolved {
        response_id,
        provider: decoded
            .custom_llm_provider
            .flatten()
            .filter(|provider| !provider.is_empty())
            .or_else(|| call.custom_llm_provider.clone()),
    }
}

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Display;

use betterlite_config::sections::Section as _;
use betterlite_config::sections::openai_credentials::Raw as CredentialsRaw;
use betterlite_config::sections::openai_provider::Raw as ProviderRaw;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::openai_base_config::{CatalogCapabilities, Headers, Object};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_provider::openai_dispatch::OpenAiRequest;
use betterlite_provider::openai_dispatch::call::Composition;
use betterlite_provider::openai_dispatch::dispatch::dispatch;
use betterlite_provider::openai_dispatch::fixtures::answer::FixtureAnswer;
use betterlite_provider::openai_dispatch::fixtures::logger::FixtureLogger;
use betterlite_provider::openai_dispatch::fixtures::text_transport::{
    FixtureTextSource, TextReply,
};
use betterlite_provider::openai_dispatch::transport::Deps;
use betterlite_provider::operation::OperationRequest;
use betterlite_streaming::stream_chunk_shaping::creation::StreamInstant;
use secrecy::SecretString;
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::Value;
use uuid::Uuid;

use super::call::operation;
use super::views::{error_status, logs_view, reply_view, requests_view};

const DEFAULT_STATUS: u16 = 200;

pub(crate) fn failure<M>(class: &str, message: M) -> Box<HelperError>
where
    M: Display,
{
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_string(),
    })
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Operation {
    Chat,
    Embedding,
    Text,
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct Globals {
    #[serde(default)]
    pub(super) api_base: Option<String>,
    #[serde(default)]
    pub(super) api_key: Option<String>,
    #[serde(default)]
    pub(super) openai_key: Option<String>,
    #[serde(default)]
    pub(super) organization: Option<String>,
}

// One call the Python helper can make, keyed the way LiteLLM reads its own arguments
#[derive(Debug, Deserialize)]
pub(crate) struct Case {
    #[serde(rename = "case")]
    pub(super) operation: Operation,
    pub(super) model: String,
    #[serde(default)]
    pub(super) api_base: Option<String>,
    #[serde(default)]
    pub(super) api_key: Option<String>,
    #[serde(default)]
    pub(super) organization: Option<String>,
    #[serde(default)]
    pub(super) headers: Option<Headers>,
    #[serde(default)]
    pub(super) extra_headers: Option<Headers>,
    #[serde(default)]
    pub(super) metadata: Option<Object>,
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) globals: Globals,
    #[serde(default)]
    pub(super) preview: bool,
    #[serde(default)]
    pub(super) chat_defaults: Option<Object>,
    #[serde(default)]
    pub(super) text_defaults: Option<Object>,
    #[serde(default)]
    pub(super) messages: Vec<Value>,
    #[serde(default)]
    pub(super) optional_params: Object,
    #[serde(default = "default_status")]
    pub(super) status: u16,
    pub(super) reply: Value,
    #[serde(default)]
    pub(super) text_completion: bool,
}

const fn default_status() -> u16 {
    DEFAULT_STATUS
}

// The clock a projected text reply stamps its created field with, fixed so a case can compare it
const fn fixed_now() -> StreamInstant {
    StreamInstant {
        seconds: 1_700_000_000,
        timestamp: 1_700_000_000.0,
    }
}

pub(crate) async fn dispatch_case(case: &Case) -> HelperResult {
    let composition = composition(case)?;
    let operation = operation(case)?;
    let reply = serde_json::to_vec(&case.reply).map_err(|error| failure("ValueError", error))?;
    // the same fixture body answers either transport, and only the branch this operation runs reads it
    let answer = FixtureAnswer::new(case.status, reply.clone());
    let text = FixtureTextSource::new(TextReply::new(case.status, reply));
    let mut logger = FixtureLogger::default();
    let result = dispatch(
        OperationRequest::OpenAi(OpenAiRequest),
        operation,
        &composition,
        Deps {
            answer: &answer,
            text: &text,
            now: fixed_now,
            // the identifier a projected text reply names itself with, fixed so a case can compare it
            next_id: || Uuid::from_u128(1),
        },
        &mut logger,
    )
    .await;
    let requests = requests_view(case, &answer, &text)?;
    let logs = logs_view(&logger)?;
    match result {
        Ok(outcome) => Ok(serde_json::json!({
            "requests": requests,
            "reply": reply_view(outcome)?,
            "error": Value::Null,
            "logs": logs,
        })),
        Err(error) => Ok(serde_json::json!({
            "requests": requests,
            "reply": Value::Null,
            "error": serde_json::json!({"status": error_status(&error)}),
            "logs": logs,
        })),
    }
}

// The composition is the real config sections over the case's environment, so the flags load the way main.py loads them
fn composition(case: &Case) -> Result<Composition, Box<HelperError>> {
    let credentials = CredentialsRaw::default()
        .convert(&case.environment)
        .or_config_error()?;
    let mut provider = ProviderRaw::default()
        .convert(&case.environment)
        .or_config_error()?;
    provider.enable_preview_features = case.preview;
    Ok(Composition {
        credentials,
        provider,
        globals: LiteLlmGlobals {
            api_base: case.globals.api_base.clone(),
            api_key: case.globals.api_key.clone().map(SecretString::from),
            openai_key: case.globals.openai_key.clone().map(SecretString::from),
            organization: case.globals.organization.clone(),
        },
        // the case never sets litellm.headers, so only the call's own maps name the headers
        globals_headers: None,
        env_api_key: case
            .environment
            .get("OPENAI_API_KEY")
            .filter(|value| !value.is_empty())
            .map(|value| SecretString::from(value.clone())),
        chat_defaults: defaults(case.chat_defaults.as_ref())?,
        text_defaults: defaults(case.text_defaults.as_ref())?,
        catalog: CatalogCapabilities {
            chat_models: BTreeSet::new(),
            text_models: BTreeSet::new(),
        },
    })
}

fn defaults<T>(fields: Option<&Object>) -> Result<T, Box<HelperError>>
where
    T: Default + DeserializeOwned,
{
    let Some(object) = fields else {
        return Ok(T::default());
    };
    serde_json::from_value(Value::Object(object.clone()))
        .map_err(|error| failure("ValueError", error))
}

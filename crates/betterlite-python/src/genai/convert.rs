use betterlite_core::google_genai::streaming::GenAiStreamError;
use betterlite_error::error::{ErrorClass, class_message};
use betterlite_models::decisions::Json as DecisionJson;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_sdk::call_files::{CallFilesError, files_from_payload};
use betterlite_sdk::call_setup::RequestMap;
use betterlite_sdk::call_wrapper::error::{class_name, class_status};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::TryFromFloatSecsError;
mod tests;
// The session text: absent means the process environment, the snapshot LiteLLM reads its globals from at import.
#[derive(Default, Deserialize)]
#[serde(default)]
pub(crate) struct SessionWire {
    pub(crate) environment: Option<BTreeMap<String, String>>,
    pub(crate) api_key: Option<String>,
}
#[derive(Debug, thiserror::Error)]
pub(crate) enum ConvertError {
    #[error("the native request text is not JSON of the expected shape: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the request timeout is not a number of seconds: {source}")]
    Timeout {
        #[source]
        source: TryFromFloatSecsError,
    },
    #[error("the request carries a file the generate content call has no place for")]
    Files,
    #[error(transparent)]
    CallFiles(#[from] CallFilesError),
}
// the one boundary every native method crosses: the text into the shape that method takes
pub(crate) fn parse<'text, T>(text: &'text str) -> Result<T, ConvertError>
where
    T: Deserialize<'text>,
{
    match serde_json::from_str(text) {
        Ok(value) => Ok(value),
        Err(source) => Err(ConvertError::Json { source }),
    }
}
// B6040 encodes a file as a $file marker with its bytes beside the text, and a generate content body has no part for one
pub(crate) fn reject_files(text: &str, blobs: &mut [Vec<u8>]) -> Result<(), ConvertError> {
    let payload: RequestMap = parse(text)?;
    if files_from_payload(&payload, blobs)?.is_empty() {
        return Ok(());
    }
    Err(ConvertError::Files)
}
// LiteLLM's logging state carries the models crate's Json, which keeps key order the way the prompts one does
#[must_use]
pub(crate) fn decision_json(value: &Json) -> DecisionJson {
    match value {
        Json::Null => DecisionJson::Null,
        Json::Bool(flag) => DecisionJson::Bool(*flag),
        Json::Number(number) => DecisionJson::Number(number.clone()),
        Json::Text(text) => DecisionJson::Text(text.clone()),
        Json::List(items) => DecisionJson::List(items.iter().map(decision_json).collect()),
        Json::Object(fields) => DecisionJson::Object(decision_object(fields)),
    }
}
#[must_use]
pub(crate) fn decision_object(object: &Object) -> IndexMap<String, DecisionJson> {
    object
        .iter()
        .map(|(name, value)| (name.clone(), decision_json(value)))
        .collect()
}
// the JSON text a NativeGenaiError carries: LiteLLM's exception name and the fields its class keeps
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct NativeErrorPayload {
    #[serde(rename = "type")]
    pub(crate) kind: String,
    pub(crate) message: String,
    pub(crate) status_code: Option<u16>,
    pub(crate) llm_provider: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) body: Option<Json>,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) num_retries: Option<i64>,
    pub(crate) max_retries: Option<i64>,
}
impl NativeErrorPayload {
    #[must_use]
    pub(crate) fn from_class(class: &ErrorClass) -> Self {
        let metadata = class.metadata();
        let mut headers = BTreeMap::new();
        if let ErrorClass::RateLimit(details) = class {
            headers.clone_from(&details.headers);
        }
        Self {
            kind: class_name(class).to_owned(),
            message: class_message(class),
            // a plain Exception carries no status_code; every LiteLLM error class does
            status_code: class_status(class),
            llm_provider: metadata.provider,
            model: metadata.model,
            body: None,
            headers,
            num_retries: None,
            max_retries: metadata.retry.max_retries.map(i64::from),
        }
    }
    // a stream that fails mid-iteration raises the provider's own exception in LiteLLM, and no class of the table fits
    #[must_use]
    pub(crate) fn from_stream(error: &GenAiStreamError) -> Self {
        Self {
            kind: String::from("Exception"),
            message: error.to_string(),
            status_code: None,
            llm_provider: None,
            model: None,
            body: None,
            headers: BTreeMap::new(),
            num_retries: None,
            max_retries: None,
        }
    }
}

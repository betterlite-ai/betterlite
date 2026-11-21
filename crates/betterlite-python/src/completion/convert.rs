use betterlite_error::error::{ErrorClass, class_message};
use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_models::controls::optional_params::OptionalParams;
use betterlite_models::decisions::Json;
use betterlite_sdk::call_setup::RequestMap;
use betterlite_sdk::call_setup::correlation::CallOutcome;
use betterlite_sdk::call_wrapper::error::{CallError, class_name, class_status};
use betterlite_sdk::completion_request::args::{CompletionFlags, CompletionSettings};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
mod tests;
// The one text a native method takes: the call's arguments, the settings snapshot and the session's own state.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct CallWire {
    pub(crate) arguments: RequestMap,
    pub(crate) settings: SettingsWire,
}
// The Python-side settings snapshot; every name is one CompletionSettings reads, absent taking LiteLLM's default.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct SettingsWire {
    pub(crate) drop_params: Option<bool>,
    pub(crate) model_fallbacks: Option<Vec<Json>>,
    pub(crate) model_alias_map: BTreeMap<String, String>,
    pub(crate) num_retries: Option<i64>,
    pub(crate) request_timeout: Option<f64>,
    pub(crate) api_key: Option<SecretString>,
    pub(crate) api_base: Option<String>,
    pub(crate) route_all_to_responses: bool,
    pub(crate) proxy_headers: Option<AdditionalHeaders>,
    pub(crate) optional: OptionalParams,
    pub(crate) add_function_to_prompt: bool,
    pub(crate) disable_stop_sequence_limit: bool,
    pub(crate) medium_thinking_budget: Option<u32>,
}
impl SettingsWire {
    pub(crate) fn into_settings(self, medium_thinking_budget: u32) -> CompletionSettings {
        CompletionSettings {
            drop_params: self.drop_params,
            model_fallbacks: self.model_fallbacks,
            model_alias_map: self.model_alias_map,
            num_retries: self.num_retries,
            request_timeout: self.request_timeout,
            api_key: self.api_key,
            api_base: self.api_base,
            route_all_to_responses: self.route_all_to_responses,
            proxy_headers: self.proxy_headers.unwrap_or_default(),
            optional: self.optional,
            flags: CompletionFlags {
                add_function_to_prompt: self.add_function_to_prompt,
                disable_stop_sequence_limit: self.disable_stop_sequence_limit,
            },
            medium_thinking_budget: self
                .medium_thinking_budget
                .unwrap_or(medium_thinking_budget),
        }
    }
}
// count_tokens args: the model, the messages, the tools and the system prompt LiteLLM counts.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct TokenCall {
    pub(crate) model: String,
    pub(crate) messages: Option<Vec<Json>>,
    pub(crate) tools: Option<Vec<Json>>,
    pub(crate) system: Option<String>,
    pub(crate) api_key: Option<SecretString>,
    pub(crate) api_base: Option<String>,
}
// check_valid_key takes the model and the key it proves; valid_model takes only the model.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct ProbeCall {
    pub(crate) model: String,
    pub(crate) api_key: Option<SecretString>,
}
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct SessionWire {
    // absent means the process environment, the snapshot LiteLLM reads its globals from at import
    pub(crate) environment: Option<BTreeMap<String, String>>,
    pub(crate) medium_thinking_budget: Option<u32>,
}
#[derive(Debug, thiserror::Error)]
pub(crate) enum ConvertError {
    #[error("the native request text is not JSON of the expected shape: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the answer could not be written as JSON: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
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
// What a finished native call hands to Python: JSON text for a response, chunk texts for a stream.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum OutcomeParts {
    Response(String),
    Stream(Vec<String>),
}
pub(crate) fn outcome_parts(outcome: CallOutcome) -> Result<OutcomeParts, ConvertError> {
    let payload = match outcome {
        CallOutcome::Response(json) => {
            let text =
                serde_json::to_string(&json).map_err(|source| ConvertError::Encode { source })?;
            return Ok(OutcomeParts::Response(text));
        }
        CallOutcome::ChatStream(stream) => stream.model_response,
        CallOutcome::ResponsesStream(json) => json,
    };
    // the SDK buffers the provider's payloads into a list; a driver that kept one payload hands it back whole
    let chunks = match payload {
        Json::List(items) => items,
        single => vec![single],
    };
    let mut texts = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        let text =
            serde_json::to_string(&chunk).map_err(|source| ConvertError::Encode { source })?;
        texts.push(text);
    }
    Ok(OutcomeParts::Stream(texts))
}
// The JSON text a NativeError carries: LiteLLM's exception name and the fields its class keeps.
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
    pub(crate) fn from_call_error(error: &CallError) -> Self {
        let metadata = error.class.metadata();
        let mut headers = BTreeMap::new();
        if let ErrorClass::RateLimit(details) = &error.class {
            headers.clone_from(&details.headers);
        }
        Self {
            kind: class_name(&error.class).to_owned(),
            message: class_message(&error.class),
            // a plain Exception carries no status_code; every LiteLLM error class does
            status_code: class_status(&error.class),
            llm_provider: metadata.provider,
            model: metadata.model,
            body: None,
            headers,
            num_retries: error.num_retries,
            max_retries: metadata.retry.max_retries.map(i64::from),
        }
    }
}

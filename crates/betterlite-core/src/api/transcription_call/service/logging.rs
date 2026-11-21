use std::sync::Mutex;
use std::sync::PoisonError;

use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_logging::logging_state::updates::{CallUpdate, update_environment_variables};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::transcription_call::call::{SuppliedClientKind, TranscriptionCall};
use crate::api::transcription_call::decisions_json;
use crate::api::transcription_call::plan::Plan;

// model_call_details["client"], then update_environment_variables with optional_params={} and the kwargs
pub(super) fn log(
    logging: Option<&Mutex<LoggingState>>,
    call: &TranscriptionCall,
    plan: &Plan,
) -> Result<(), LoggingStateError> {
    let Some(shared) = logging else {
        return Ok(());
    };
    let client = client_name(call.client);
    let mut litellm_params = IndexMap::new();
    for (name, value) in &plan.kwargs {
        litellm_params.insert(name.clone(), decisions_json(value));
    }
    for (name, value) in metadata_defaults() {
        litellm_params.entry(String::from(name)).or_insert(value);
    }
    let mut additional_params = IndexMap::new();
    additional_params.insert(
        String::from("custom_llm_provider"),
        Json::Text(plan.slug.clone()),
    );
    write_state(
        &mut shared.lock().unwrap_or_else(PoisonError::into_inner),
        client,
        &litellm_params,
        CallUpdate {
            model: Some(plan.model.clone()),
            user: call.user.clone(),
            optional_params: IndexMap::new(),
            additional_params,
        },
    )
}

// the two writes happen under one short lock, once every parameter map is built
fn write_state(
    state: &mut LoggingState,
    client: String,
    litellm_params: &IndexMap<String, Json>,
    update: CallUpdate,
) -> Result<(), LoggingStateError> {
    state
        .model_call_details
        .insert(String::from("client"), Json::Text(client));
    update_environment_variables(state, litellm_params, update)
}

// the update names these before **kwargs, so a caller value replaces the default
fn metadata_defaults() -> [(&'static str, Json); 6] {
    [
        ("litellm_call_id", Json::Null),
        ("proxy_server_request", Json::Null),
        ("model_info", Json::Null),
        ("metadata", Json::Null),
        ("preset_cache_key", Json::Null),
        ("stream_response", Json::Object(IndexMap::new())),
    ]
}

// str(client): a supplied client's class name, and Python's None text when the caller passed none
fn client_name(client: Option<SuppliedClientKind>) -> String {
    match client {
        None => String::from("None"),
        Some(SuppliedClientKind::OpenAi) => String::from("openai.OpenAI"),
        Some(SuppliedClientKind::AsyncOpenAi) => String::from("openai.AsyncOpenAI"),
        Some(SuppliedClientKind::AzureOpenAi) => String::from("openai.AzureOpenAI"),
        Some(SuppliedClientKind::AsyncAzureOpenAi) => String::from("openai.AsyncAzureOpenAI"),
        Some(SuppliedClientKind::HttpHandler) => {
            String::from("litellm.llms.custom_httpx.http_handler.HTTPHandler")
        }
        Some(SuppliedClientKind::AsyncHttpHandler) => {
            String::from("litellm.llms.custom_httpx.http_handler.AsyncHTTPHandler")
        }
    }
}

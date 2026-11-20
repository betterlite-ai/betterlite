use std::sync::PoisonError;

use betterlite_logging::logging_state::LoggingStateError;
use betterlite_logging::logging_state::updates::{CallUpdate, update_environment_variables};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::super::call::{SuppliedClientKind, TranscriptionCall};
use super::super::plan::Plan;
use super::Transcriptions;

impl<Dispatch> Transcriptions<'_, Dispatch> {
    // model_call_details["client"], then update_environment_variables with optional_params={} and the kwargs
    pub(super) fn log(
        &self,
        call: &TranscriptionCall,
        plan: &Plan,
    ) -> Result<(), LoggingStateError> {
        let Some(logging) = self.logging else {
            return Ok(());
        };
        let mut state = logging.lock().unwrap_or_else(PoisonError::into_inner);
        state
            .model_call_details
            .insert(String::from("client"), Json::Text(client_name(call.client)));
        let mut litellm_params = IndexMap::new();
        for (name, value) in &plan.kwargs {
            litellm_params.insert(name.clone(), super::super::decisions_json(value));
        }
        for (name, value) in metadata_defaults() {
            litellm_params.entry(String::from(name)).or_insert(value);
        }
        let mut additional_params = IndexMap::new();
        additional_params.insert(
            String::from("custom_llm_provider"),
            Json::Text(plan.slug.clone()),
        );
        update_environment_variables(
            &mut state,
            &litellm_params,
            CallUpdate {
                model: Some(plan.model.clone()),
                user: call.user.clone(),
                optional_params: IndexMap::new(),
                additional_params,
            },
        )
    }
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

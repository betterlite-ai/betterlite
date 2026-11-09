use std::sync::{Arc, Mutex, PoisonError};

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::rubrik::prompt_payload::PromptModerationRequest;
use crate::rubrik::request_data::{RequestContextSource, TrackedField};

use super::error::ModerationError;

// One Logging object is shared by the request, its success event and the failure hook, so the block flag lives behind a Mutex in an Arc.
#[derive(Debug)]
pub struct LoggingObject {
    pub model_call_details: Mutex<IndexMap<String, Json>>,
}

#[derive(Clone, Debug, Default)]
pub enum LoggingObjectStash {
    #[default]
    Missing,
    Present(Option<Arc<LoggingObject>>),
}

// The request mapping LiteLLM carries as dict[str, Any]; the stashed logging object rides beside the JSON fields because it is not one.
#[derive(Clone, Debug, Default)]
pub struct RequestData {
    pub fields: IndexMap<String, Json>,
    pub logging_obj: LoggingObjectStash,
}

#[must_use]
pub fn call_details(logging_obj: Option<&Arc<LoggingObject>>) -> IndexMap<String, Json> {
    let Some(present) = logging_obj else {
        return IndexMap::new();
    };
    let details = present
        .model_call_details
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    details.clone()
}

pub fn text_field<'fields>(
    fields: &'fields IndexMap<String, Json>,
    field: &'static str,
) -> Result<Option<&'fields str>, ModerationError> {
    match fields.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text)),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(ModerationError::FieldType { field })
        }
    }
}

// The call-details half of the after_completion envelope's request.
pub fn request_context(
    fields: &IndexMap<String, Json>,
) -> Result<RequestContextSource, ModerationError> {
    let messages = match fields.get("messages") {
        None | Some(Json::Null) => None,
        Some(messages) => Some(messages.clone()),
    };
    let model = text_field(fields, "model")?.map(str::to_owned);
    let optional_params_tools = match fields.get("optional_params") {
        None | Some(Json::Null) => None,
        Some(Json::Object(params)) => params.get("tools").cloned(),
        Some(_) => {
            return Err(ModerationError::FieldType {
                field: "optional_params",
            });
        }
    };
    Ok(RequestContextSource {
        messages,
        model,
        tools: TrackedField::Missing,
        optional_params_tools,
    })
}

// The request-data half: tools are read by presence so an explicit empty list still wins.
#[must_use]
pub fn request_context_source(fields: &IndexMap<String, Json>) -> RequestContextSource {
    let tools = fields.get("tools").map_or_else(
        || TrackedField::Missing,
        |tools| TrackedField::Present(tools.clone()),
    );
    RequestContextSource {
        messages: None,
        model: None,
        tools,
        optional_params_tools: None,
    }
}

pub fn prompt_request(
    fields: &IndexMap<String, Json>,
) -> Result<PromptModerationRequest<'_>, ModerationError> {
    Ok(PromptModerationRequest {
        model: text_field(fields, "model")?,
        user: text_field(fields, "user")?,
        correlation_key: text_field(fields, "correlation_key")?,
        litellm_call_id: text_field(fields, "litellm_call_id")?,
    })
}

// The after_completion envelope's request half: LiteLLM sends all three keys, nulls included, whenever either source carries any data.
#[must_use]
pub fn request_data_json(
    messages: Option<Json>,
    model: Option<String>,
    tools: Option<Json>,
) -> Json {
    Json::Object(IndexMap::from([
        ("messages".to_owned(), messages.unwrap_or(Json::Null)),
        ("model".to_owned(), model.map_or(Json::Null, Json::Text)),
        ("tools".to_owned(), tools.unwrap_or(Json::Null)),
    ]))
}

pub fn stash_block_context(
    logging_obj: Option<&Arc<LoggingObject>>,
    request_data: &mut RequestData,
) {
    let Some(present) = logging_obj else {
        let call_id = text_field(&request_data.fields, "litellm_call_id")
            .ok()
            .flatten()
            .unwrap_or("None");
        tracing::error!(
            litellm_call_id = %call_id,
            "Rubrik: moderation block fired with logging_obj=None for litellm_call_id={call_id}; cannot suppress success event or attach failure payload."
        );
        request_data.logging_obj = LoggingObjectStash::Present(None);
        return;
    };
    present
        .model_call_details
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert("_rubrik_blocked".to_owned(), Json::Bool(true));
    request_data.logging_obj = LoggingObjectStash::Present(Some(Arc::clone(present)));
}

use std::sync::{Arc, Mutex, PoisonError};

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::rubrik::prompt_payload::PromptModerationRequest;
use crate::rubrik::request_data::{RequestContextSource, ResponseSource, TrackedField};

use super::error::ModerationError;

// LiteLLM shares one Logging object across the request, its success event and the failure hook;
// a Mutex behind the Arc lets the moderation write the block flag while those paths read it.
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

#[derive(Clone, Debug)]
pub struct RequestData {
    pub litellm_call_id: Option<String>,
    pub model: Option<String>,
    pub user: Option<String>,
    pub correlation_key: Option<String>,
    pub tools: TrackedField<Json>,
    pub response: ResponseSource,
    pub logging_obj: LoggingObjectStash,
}

impl Default for RequestData {
    fn default() -> Self {
        Self {
            litellm_call_id: None,
            model: None,
            user: None,
            correlation_key: None,
            tools: TrackedField::Missing,
            response: ResponseSource::Absent,
            logging_obj: LoggingObjectStash::Missing,
        }
    }
}

impl RequestData {
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.litellm_call_id.is_none()
            && self.model.is_none()
            && self.user.is_none()
            && self.correlation_key.is_none()
            && matches!(self.tools, TrackedField::Missing)
            && matches!(self.response, ResponseSource::Absent)
            && matches!(self.logging_obj, LoggingObjectStash::Missing)
    }

    #[must_use]
    pub fn prompt_request(&self) -> PromptModerationRequest<'_> {
        PromptModerationRequest {
            model: self.model.as_deref(),
            user: self.user.as_deref(),
            correlation_key: self.correlation_key.as_deref(),
            litellm_call_id: self.litellm_call_id.as_deref(),
        }
    }

    #[must_use]
    pub fn request_context_source(&self) -> RequestContextSource {
        RequestContextSource {
            messages: None,
            model: None,
            tools: self.tools.clone(),
            optional_params_tools: None,
        }
    }
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
        Some(_) => Err(ModerationError::FieldType { field }),
    }
}

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

// The request half of the after_completion envelope: LiteLLM sends all three keys, nulls included,
// whenever either source carries any data.
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
        let call_id = request_data.litellm_call_id.as_deref().unwrap_or("None");
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

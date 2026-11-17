use std::sync::Arc;

use betterlite_logging::logging_privacy::callbacks::TRUSTED_FIELDS;
use betterlite_logging::logging_privacy::internal::{self, Forwarding, Origin};
use betterlite_logging::logging_privacy::request::{Flag, Metadata, Params, PrivacyRequest};
use betterlite_logging::logging_privacy::{CallbackMetadataError, PrivacyContext};
use betterlite_models::decisions::Json;
use betterlite_openai::chat::ChatMessage;
use betterlite_router::semantic_router::error::RouterError;
use betterlite_router::semantic_router::service::{
    AutoRouter, CallerContext, RouteDecision, RouteEmbedder,
};
use indexmap::IndexMap;

mod tests;

// the request fields LiteLLM's auto router reads, in the shape its caller passes them
pub struct CallerRequest<'request> {
    pub metadata: Option<&'request Json>,
    pub litellm_metadata: Option<&'request Json>,
    pub litellm_params_metadata: Option<&'request Json>,
    pub litellm_session_id: Option<&'request Json>,
    pub litellm_trace_id: Option<&'request Json>,
    pub turn_off_message_logging: Option<&'request Json>,
}

// the caller identity, flag and session the embedding sub-call carries, as LiteLLM forwards them
pub fn caller_context(
    request: &CallerRequest<'_>,
    context: PrivacyContext,
) -> Result<CallerContext, CallbackMetadataError> {
    let params = privacy_request(request)?.dynamic_params(context)?;
    Ok(CallerContext {
        metadata: forwarded(request.metadata, "metadata")?,
        litellm_metadata: forwarded(request.litellm_metadata, "litellm_metadata")?,
        turn_off_message_logging: match params.get("turn_off_message_logging") {
            Some(Json::Bool(disabled)) => Some(*disabled),
            _ => None,
        },
        litellm_session_id: session_text(request.litellm_session_id),
        litellm_trace_id: session_text(request.litellm_trace_id),
    })
}

// the body LiteLLM records on the embedding sub-call: the whole text, not the clamped one
#[must_use]
pub fn proxy_server_request(embedding_model: &str, text: &str) -> Json {
    Json::Object(IndexMap::from([(
        "body".to_owned(),
        Json::Object(IndexMap::from([
            ("model".to_owned(), Json::Text(embedding_model.to_owned())),
            (
                "input".to_owned(),
                Json::List(vec![Json::Text(text.to_owned())]),
            ),
        ])),
    )]))
}

// flag resolution and metadata forwarding both fall inside LiteLLM's fallback, so unusable metadata still takes the default
pub async fn decide<E>(
    router: &AutoRouter<E>,
    messages: Option<&[ChatMessage]>,
    request: &CallerRequest<'_>,
    context: PrivacyContext,
) -> Result<Option<RouteDecision>, Arc<RouterError<E::Error>>>
where
    E: RouteEmbedder + 'static,
{
    router
        .decide_with(messages, || match caller_context(request, context) {
            Ok(caller) => Some(caller),
            Err(error) => {
                tracing::warn!(%error, "semantic routing metadata is unusable, using the default model");
                None
            }
        })
        .await
}

fn privacy_request(request: &CallerRequest<'_>) -> Result<PrivacyRequest, CallbackMetadataError> {
    Ok(PrivacyRequest {
        direct: Metadata {
            turn_off_message_logging: request
                .turn_off_message_logging
                .map(flag)
                .transpose()?
                .flatten(),
            redact_user_api_key_info: None,
            headers: IndexMap::new(),
            callback_params: IndexMap::new(),
        },
        litellm_params: Some(Params {
            metadata: request
                .litellm_params_metadata
                .filter(|slot| !matches!(slot, Json::Null))
                .map(|slot| slot_metadata(slot, "litellm_params.metadata"))
                .transpose()?,
        }),
        litellm_metadata: request
            .litellm_metadata
            .filter(|slot| !matches!(slot, Json::Null))
            .map(|slot| slot_metadata(slot, "litellm_metadata"))
            .transpose()?,
        metadata: request
            .metadata
            .filter(|slot| !matches!(slot, Json::Null))
            .map(|slot| slot_metadata(slot, "metadata"))
            .transpose()?,
    })
}

fn flag(value: &Json) -> Result<Option<Flag>, CallbackMetadataError> {
    match value {
        Json::Bool(enabled) => Ok(Some(Flag::Boolean(*enabled))),
        Json::Text(text) => Ok(Some(Flag::Text(text.clone()))),
        Json::Null => Ok(None),
        Json::Number(_) | Json::List(_) | Json::Object(_) => {
            Err(CallbackMetadataError::InvalidField {
                field: "turn_off_message_logging".to_owned(),
            })
        }
    }
}

// trusted internal fields are ignored here as LiteLLM's dynamic param reader ignores them
fn slot_metadata(slot: &Json, field: &str) -> Result<Metadata, CallbackMetadataError> {
    let Json::Object(fields) = slot else {
        return Err(CallbackMetadataError::InvalidField {
            field: field.to_owned(),
        });
    };
    let mut callback_params = IndexMap::new();
    let mut turn_off_message_logging = None;
    for (name, value) in fields {
        if name == "turn_off_message_logging" {
            turn_off_message_logging = flag(value)?;
        } else if !TRUSTED_FIELDS.contains(&name.as_str()) {
            callback_params.insert(name.clone(), value.clone());
        } else {
            // a trusted internal field is ignored here, as LiteLLM's dynamic param reader ignores it
        }
    }
    Ok(Metadata {
        turn_off_message_logging,
        redact_user_api_key_info: None,
        headers: IndexMap::new(),
        callback_params,
    })
}

fn forwarded(
    metadata: Option<&Json>,
    field: &str,
) -> Result<IndexMap<String, Json>, CallbackMetadataError> {
    match metadata {
        None | Some(Json::Null) => Ok(IndexMap::new()),
        Some(Json::Object(fields)) => {
            internal::forward(fields, Origin::AutorouterClassifier, Forwarding::Attached)
        }
        Some(_) => Err(CallbackMetadataError::InvalidField {
            field: field.to_owned(),
        }),
    }
}

fn session_text(value: Option<&Json>) -> Option<String> {
    match value {
        Some(Json::Text(text)) => Some(text.clone()),
        _ => None,
    }
}

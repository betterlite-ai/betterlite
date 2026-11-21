use betterlite_models::decisions::Json;
use futures_util::future::BoxFuture;

use crate::api::responses_input_content::is_truthy;
use crate::shadow_evidence::{chat_text, responses_text};

use super::Object;
use super::error::ShadowRequestError;

pub mod anthropic;
pub mod chat;
pub mod environment;
pub mod responses;

mod tests;

const MODEL: &str = "model";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    Chat,
    AnthropicMessages,
    Responses,
}

pub type ChatRequestFuture<'request> = BoxFuture<'request, Result<Object, ShadowRequestError>>;
pub type ChatRequestFn =
    for<'request> fn(&'request Object, &'request Object) -> ChatRequestFuture<'request>;

pub struct SurfaceOps {
    pub chat_request: ChatRequestFn,
    pub final_text: fn(&Json) -> String,
    pub wire_params: bool,
    pub surface: Surface,
}

static CHAT_OPS: SurfaceOps = SurfaceOps {
    chat_request: chat::chat_request,
    final_text: chat_text::final_text,
    wire_params: false,
    surface: Surface::Chat,
};

static ANTHROPIC_OPS: SurfaceOps = SurfaceOps {
    chat_request: anthropic::chat_request,
    final_text: chat_text::final_text,
    wire_params: true,
    surface: Surface::AnthropicMessages,
};

static RESPONSES_OPS: SurfaceOps = SurfaceOps {
    chat_request: responses::chat_request,
    final_text: responses_text::final_text,
    wire_params: true,
    surface: Surface::Responses,
};

// membership in this table is the sampling allowlist; an unknown call type fails closed
#[must_use]
pub fn surface_ops(call_type: &str) -> Option<&'static SurfaceOps> {
    match call_type {
        "completion" | "acompletion" => Some(&CHAT_OPS),
        "anthropic_messages" => Some(&ANTHROPIC_OPS),
        "aresponses" | "responses" => Some(&RESPONSES_OPS),
        _ => None,
    }
}

// the proxy's snapshot of the client's wire body, the only source the wire_params surfaces trust
#[must_use]
pub fn proxy_wire_body(kwargs: &Object) -> Object {
    let Some(Json::Object(parameters)) = kwargs.get("litellm_params") else {
        return Object::new();
    };
    let Some(Json::Object(request)) = parameters.get("proxy_server_request") else {
        return Object::new();
    };
    match request.get("body") {
        Some(Json::Object(body)) => body.clone(),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Object::new()
        }
    }
}

// the shadow call's model: missing or null is the empty string, and any other non-text value is a rejected shape
fn model(kwargs: &Object) -> Result<String, ShadowRequestError> {
    match kwargs.get(MODEL) {
        None | Some(Json::Null) => Ok(String::new()),
        Some(Json::Text(text)) => Ok(text.clone()),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(ShadowRequestError::ModelNotText)
        }
    }
}

// LiteLLM's `or`: the fallback is read whenever the first value would be falsy, and a falsy fallback is kept
pub fn or_value<'value>(
    primary: Option<&'value Json>,
    fallback: Option<&'value Json>,
) -> Result<Option<&'value Json>, ShadowRequestError> {
    let truthy = match primary {
        Some(value) => is_truthy(Some(&serde_json::to_value(value)?)),
        None => false,
    };
    Ok(if truthy { primary } else { fallback })
}

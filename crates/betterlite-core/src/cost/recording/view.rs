use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::cost::context::CostResponse;
use crate::cost::prepare::{DictionaryResponse, ResponseView, TypedResponse};
use crate::cost::usage::{BillingResponse, BillingUsage, ResponseKind};

const TERMINAL_EVENTS: [&str; 3] = [
    "response.completed",
    "response.incomplete",
    "response.failed",
];

#[derive(Debug, thiserror::Error)]
pub(super) enum JsonResponseError {
    #[error("the response usage must be an object or null")]
    Usage,
    #[error("the response _hidden_params must be an object or null")]
    Hidden,
    #[error("the terminal event must carry a response object")]
    Response,
}

// A mapping body exposes only its keys, the same view the dictionary cost path reads.
pub(super) fn body_view(body: &Json) -> Result<Option<ResponseView<'_>>, JsonResponseError> {
    let Some(fields) = object(body) else {
        return Ok(None);
    };
    Ok(Some(ResponseView::Dictionary(DictionaryResponse {
        model: text(fields, "model"),
        usage: usage_of(fields)?,
        service_tier: text(fields, "service_tier"),
    })))
}

// Ports the terminal-event unwrap: only completion, incomplete and failure events carry the response.
pub(super) fn event_view(event: &Json) -> Result<Option<ResponseView<'_>>, JsonResponseError> {
    let Some(fields) = object(event) else {
        return Ok(None);
    };
    let Some(event_type) = text(fields, "type") else {
        return Ok(None);
    };
    if !TERMINAL_EVENTS.contains(&event_type) {
        return Ok(None);
    }
    let Some(inner) = fields.get("response") else {
        return Err(JsonResponseError::Response);
    };
    let Some(response) = object(inner) else {
        return Err(JsonResponseError::Response);
    };
    Ok(Some(ResponseView::Typed(TypedResponse {
        kind: ResponseKind::Responses,
        model: text(response, "model"),
        usage: usage_of(response)?,
        service_tier: text(response, "service_tier"),
        hidden_params: hidden(response)?,
    })))
}

// is_background_response: a mapping reads its key, every other shape carries no such attribute.
pub(super) fn background(body: &Json) -> bool {
    object(body)
        .and_then(|fields| fields.get("background"))
        .is_some_and(|value| matches!(value, Json::Bool(true)))
}

pub(super) fn hidden_params<'view>(
    view: &'view ResponseView<'_>,
) -> Option<&'view IndexMap<String, Json>> {
    match view {
        ResponseView::Typed(typed) => typed.hidden_params,
        ResponseView::Realtime { .. } | ResponseView::Dictionary(_) | ResponseView::Other => None,
    }
}

pub(super) fn hidden_text<'view>(view: &'view ResponseView<'_>, key: &str) -> Option<&'view str> {
    match hidden_params(view)?.get(key) {
        Some(Json::Text(value)) if !value.is_empty() => Some(value),
        Some(
            Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::Text(_)
            | Json::List(_)
            | Json::Object(_),
        )
        | None => None,
    }
}

pub(super) fn cost_response<'view>(
    view: &'view ResponseView<'view>,
) -> Option<CostResponse<'view>> {
    match view {
        ResponseView::Typed(typed) => Some(CostResponse::Object {
            model: typed.model,
            hidden_params: typed.hidden_params,
            response_ms: None,
        }),
        ResponseView::Dictionary(dictionary) => Some(CostResponse::Dictionary {
            model: dictionary.model,
        }),
        ResponseView::Realtime { .. } | ResponseView::Other => None,
    }
}

// The usage projection get_usage_object reads, rebuilt here because prepare's view stays private.
pub(super) fn billing_response(view: &ResponseView<'_>) -> BillingResponse<'_> {
    BillingResponse {
        kind: kind(view),
        usage: usage(view),
        service_tier: service_tier(view),
        hidden_params: hidden_params(view),
    }
}

fn kind(view: &ResponseView<'_>) -> ResponseKind {
    match view {
        ResponseView::Typed(typed) => typed.kind,
        ResponseView::Realtime { .. } | ResponseView::Dictionary(_) | ResponseView::Other => {
            ResponseKind::Other
        }
    }
}

fn usage<'usage>(view: &'usage ResponseView<'_>) -> BillingUsage<'usage> {
    match view {
        ResponseView::Realtime { usage, .. } => BillingUsage::Chat(usage),
        ResponseView::Typed(typed) => copied_usage(&typed.usage),
        ResponseView::Dictionary(dictionary) => copied_usage(&dictionary.usage),
        ResponseView::Other => BillingUsage::Absent,
    }
}

fn service_tier<'tier>(view: &'tier ResponseView<'_>) -> Option<&'tier str> {
    match view {
        ResponseView::Typed(typed) => typed.service_tier,
        ResponseView::Dictionary(dictionary) => dictionary.service_tier,
        ResponseView::Realtime { .. } | ResponseView::Other => None,
    }
}

// Rebuilding the variant keeps the reference without a Copy derive on BillingUsage.
const fn copied_usage<'usage>(usage: &'usage BillingUsage<'_>) -> BillingUsage<'usage> {
    match usage {
        BillingUsage::Absent => BillingUsage::Absent,
        BillingUsage::Null => BillingUsage::Null,
        BillingUsage::Chat(found) => BillingUsage::Chat(found),
        BillingUsage::Response(found) => BillingUsage::Response(found),
        BillingUsage::Transcription(found) => BillingUsage::Transcription(found),
        BillingUsage::DecisionsIr(found) => BillingUsage::DecisionsIr(found),
        BillingUsage::DecisionsOpenAi(found) => BillingUsage::DecisionsOpenAi(found),
        BillingUsage::Interactions(found) => BillingUsage::Interactions(found),
        BillingUsage::Mapping(found) => BillingUsage::Mapping(found),
        BillingUsage::TypedRecord(found) => BillingUsage::TypedRecord(found),
    }
}

fn object(value: &Json) -> Option<&IndexMap<String, Json>> {
    match value {
        Json::Object(fields) => Some(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

fn text<'fields>(fields: &'fields IndexMap<String, Json>, key: &str) -> Option<&'fields str> {
    match fields.get(key) {
        Some(Json::Text(value)) => Some(value),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

fn usage_of(fields: &IndexMap<String, Json>) -> Result<BillingUsage<'_>, JsonResponseError> {
    match fields.get("usage") {
        None => Ok(BillingUsage::Absent),
        Some(Json::Null) => Ok(BillingUsage::Null),
        Some(Json::Object(usage)) => Ok(BillingUsage::Mapping(usage)),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(JsonResponseError::Usage)
        }
    }
}

fn hidden(
    fields: &IndexMap<String, Json>,
) -> Result<Option<&IndexMap<String, Json>>, JsonResponseError> {
    match fields.get("_hidden_params") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(hidden)) => Ok(Some(hidden)),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(JsonResponseError::Hidden)
        }
    }
}

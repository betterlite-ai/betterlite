use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::identity::{object_value, text_value};
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
    let Some(fields) = object_value(body) else {
        return Ok(None);
    };
    Ok(Some(ResponseView::Dictionary(DictionaryResponse {
        model: fields.get("model").and_then(text_value),
        usage: usage_of(fields.get("usage"))?,
        service_tier: fields.get("service_tier").and_then(text_value),
    })))
}

// Ports the terminal-event unwrap: only completion, incomplete and failure events carry the response.
pub(super) fn event_view(event: &Json) -> Result<Option<ResponseView<'_>>, JsonResponseError> {
    let Some(fields) = object_value(event) else {
        return Ok(None);
    };
    let Some(event_type) = fields.get("type").and_then(text_value) else {
        return Ok(None);
    };
    if !TERMINAL_EVENTS.contains(&event_type) {
        return Ok(None);
    }
    let Some(response) = fields.get("response").and_then(object_value) else {
        return Err(JsonResponseError::Response);
    };
    Ok(Some(ResponseView::Typed(TypedResponse {
        kind: ResponseKind::Responses,
        model: response.get("model").and_then(text_value),
        usage: usage_of(response.get("usage"))?,
        service_tier: response.get("service_tier").and_then(text_value),
        hidden_params: hidden(response)?,
    })))
}

// is_background_response: a mapping reads its key, every other shape carries no such attribute.
pub(super) fn background(body: &Json) -> bool {
    object_value(body)
        .and_then(|fields| fields.get("background"))
        .is_some_and(|value| matches!(value, Json::Bool(true)))
}

// The variant carrying response attributes; the others expose no kind or hidden params.
pub(super) const fn typed<'view>(
    view: &'view ResponseView<'_>,
) -> Option<&'view TypedResponse<'view>> {
    if let ResponseView::Typed(typed) = view {
        return Some(typed);
    }
    None
}

pub(super) fn hidden_text<'view>(view: &'view ResponseView<'_>, key: &str) -> Option<&'view str> {
    let params = typed(view)?.hidden_params?;
    params
        .get(key)
        .and_then(text_value)
        .filter(|value| !value.is_empty())
}

pub(super) const fn cost_response<'view>(
    view: &'view ResponseView<'view>,
) -> Option<CostResponse<'view>> {
    match view {
        ResponseView::Typed(typed) => Some(CostResponse::Object {
            model: typed.model,
            hidden_params: typed.hidden_params,
            response_ms: None,
        }),
        ResponseView::Realtime { .. } => Some(CostResponse::Object {
            model: None,
            hidden_params: None,
            response_ms: None,
        }),
        ResponseView::Dictionary(dictionary) => Some(CostResponse::Dictionary {
            model: dictionary.model,
        }),
        ResponseView::Other => None,
    }
}

// The usage projection get_usage_object reads, rebuilt here because prepare's view stays private.
pub(super) fn billing_response<'view>(view: &'view ResponseView<'view>) -> BillingResponse<'view> {
    let mut billing = BillingResponse {
        kind: ResponseKind::Other,
        usage: BillingUsage::Absent,
        service_tier: None,
        hidden_params: None,
    };
    match view {
        ResponseView::Typed(typed) => {
            billing.kind = typed.kind;
            billing.usage = copied_usage(&typed.usage);
            billing.service_tier = typed.service_tier;
            billing.hidden_params = typed.hidden_params;
        }
        ResponseView::Dictionary(dictionary) => {
            billing.usage = copied_usage(&dictionary.usage);
            billing.service_tier = dictionary.service_tier;
        }
        ResponseView::Realtime { record, usage } => {
            billing.usage = BillingUsage::Chat(usage);
            billing.service_tier = record.service_tier.as_deref();
        }
        ResponseView::Other => {}
    }
    billing
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

const fn usage_of(found: Option<&Json>) -> Result<BillingUsage<'_>, JsonResponseError> {
    match found {
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
    fields
        .get("_hidden_params")
        .filter(|value| !matches!(value, Json::Null))
        .map(|value| object_value(value).ok_or(JsonResponseError::Hidden))
        .transpose()
}

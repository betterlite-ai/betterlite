use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::fallback_observations::error_info::{
    FALLBACK_ERRORS_HEADER, fallback_errors_from_headers,
};
use betterlite_router::fallback_observations::headers::attach_fallback_headers;
use betterlite_router::router_core::headers::{Response, ensure_additional_headers_dict};

use super::super::wire::{HeaderCase, HeaderFound, ResponseKind};

pub(super) fn header_found(case: &HeaderCase) -> HeaderFound {
    let response = attach_fallback_headers(
        response_for(case.kind, case.existing.as_ref()),
        case.attempted_fallbacks,
        case.errors.as_deref(),
    );
    let hidden = hidden_params(&response);
    let additional = ensure_additional_headers_dict(&hidden);
    let errors_present = additional.contains_key(FALLBACK_ERRORS_HEADER);
    let headers: JsonObject = additional
        .iter()
        .filter(|(name, _value)| name.as_str() != FALLBACK_ERRORS_HEADER)
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    HeaderFound {
        result: case.kind,
        has_hidden_params: has_hidden_params(&response),
        headers,
        errors_present,
        errors: fallback_errors_from_headers(&additional),
    }
}

fn response_for(kind: ResponseKind, existing: Option<&JsonObject>) -> Response {
    let mut hidden = JsonObject::new();
    if let Some(headers) = existing {
        hidden.insert(
            "additional_headers".to_owned(),
            Json::Object(headers.clone()),
        );
    }
    match kind {
        ResponseKind::Absent => Response::Absent,
        // a dict keeps its body and carries the hidden params under _hidden_params, which add_fallback_headers_to_response creates
        ResponseKind::Object => Response::Object(JsonObject::from_iter([(
            "_hidden_params".to_owned(),
            Json::Object(hidden),
        )])),
        ResponseKind::Attributed => Response::Attributed(hidden),
        ResponseKind::Opaque => Response::Opaque,
    }
}

fn hidden_params(response: &Response) -> JsonObject {
    match response {
        Response::Object(body) => match body.get("_hidden_params") {
            Some(Json::Object(hidden)) => hidden.clone(),
            None
            | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
                JsonObject::new()
            }
        },
        Response::Attributed(hidden) => hidden.clone(),
        Response::Absent | Response::Stream | Response::Opaque => JsonObject::new(),
    }
}

fn has_hidden_params(response: &Response) -> bool {
    match response {
        Response::Object(body) => body.contains_key("_hidden_params"),
        Response::Attributed(_) | Response::Stream => true,
        Response::Absent | Response::Opaque => false,
    }
}

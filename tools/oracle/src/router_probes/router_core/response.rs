use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::router_core::headers::{
    Response, apply_response_model_id, ensure_additional_headers, prepare_response,
    response_has_hidden_params,
};

use super::schema::{Carrier, ResponseFound, Scenario};

// prepare_response over each carrier a caller can hand the router
fn response(scenario: &Scenario) -> Response {
    match scenario.carrier {
        Carrier::Dict => {
            let mut body = JsonObject::new();
            if let Some(hidden) = &scenario.hidden {
                body.insert("_hidden_params".to_owned(), Json::Object(hidden.clone()));
            }
            Response::Object(body)
        }
        Carrier::Attributed => Response::Attributed(scenario.hidden.clone().unwrap_or_default()),
        Carrier::Stream => Response::Stream,
    }
}
pub(super) fn observe(scenario: &Scenario) -> ResponseFound {
    let Some(mut prepared) = prepare_response(response(scenario)) else {
        return ResponseFound {
            prepared: false,
            has_hidden: false,
            hidden: JsonObject::new(),
        };
    };
    ensure_additional_headers(&mut prepared);
    apply_response_model_id(&mut prepared, scenario.metadata.as_ref());
    let has_hidden = response_has_hidden_params(&prepared);
    let hidden = if has_hidden {
        prepared.hidden_params()
    } else {
        JsonObject::new()
    };
    ResponseFound {
        prepared: true,
        has_hidden,
        hidden,
    }
}

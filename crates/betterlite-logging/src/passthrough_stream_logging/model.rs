use betterlite_models::decisions::Json;
use regex::Regex;

use super::types::EndpointType;

const MODEL_PATH_PATTERN: &str = "/models/([^:]+)";
const UNKNOWN_MODEL: &str = "unknown";

#[must_use]
// ast-grep-ignore: no-long-names
pub fn extract_model_for_cost_injection(
    request_body: Option<&Json>,
    model_call_details_model: Option<&str>,
    url_route: &str,
    endpoint_type: EndpointType,
) -> Option<String> {
    if let Some(model) = request_body_model(request_body) {
        return Some(model.to_owned());
    }
    if let Some(model) = model_call_details_model.filter(|model| !model.is_empty()) {
        return Some(model.to_owned());
    }
    if endpoint_type == EndpointType::VertexAi {
        let model = extract_model_from_url(url_route);
        if model != UNKNOWN_MODEL {
            return Some(model);
        }
    }
    None
}

fn request_body_model(request_body: Option<&Json>) -> Option<&str> {
    let Json::Object(body) = request_body? else {
        return None;
    };
    match body.get("model") {
        Some(Json::Text(model)) if !model.is_empty() => Some(model),
        _ => None,
    }
}

// B3638 owns this rule in betterlite-vertex, which builds on betterlite-openai and so cannot be a dependency of this crate.
fn extract_model_from_url(url: &str) -> String {
    let Ok(pattern) = Regex::new(MODEL_PATH_PATTERN) else {
        return UNKNOWN_MODEL.to_owned();
    };
    let Some(captures) = pattern.captures(url) else {
        return UNKNOWN_MODEL.to_owned();
    };
    captures.get(1).map_or_else(
        || UNKNOWN_MODEL.to_owned(),
        |model| model.as_str().to_owned(),
    )
}

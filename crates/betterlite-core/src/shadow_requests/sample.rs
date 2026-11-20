use betterlite_models::decisions::Json;

use super::Object;
use super::surfaces::{SurfaceOps, proxy_wire_body};
use super::web_search::request_has_hosted_web_search;

mod tests;

const MESSAGES: &str = "messages";
const UNFORWARDED: [&str; 5] = ["model", "messages", "stream", "stream_options", "metadata"];

pub async fn judgeable_sample(
    surface: &SurfaceOps,
    kwargs: &Object,
    model_parameters: &Object,
    response: &Json,
) -> Option<(Vec<Object>, Object, String)> {
    let searched = if surface.wire_params {
        proxy_wire_body(kwargs)
    } else {
        model_parameters.clone()
    };
    if request_has_hosted_web_search(&searched) {
        return None;
    }
    let request = match (surface.chat_request)(kwargs, model_parameters).await {
        Ok(request) => request,
        Err(error) => {
            tracing::debug!(error = %error, "shadow eval: request normalization failed, skipping");
            return None;
        }
    };
    if request_has_hosted_web_search(&request) {
        return None;
    }
    let Some(messages) = message_objects(&request) else {
        tracing::debug!(
            "shadow eval: request normalization failed, skipping: messages is not an array of message objects"
        );
        return None;
    };
    let real_text = (surface.final_text)(response);
    if messages.is_empty() || real_text.is_empty() {
        return None;
    }
    Some((messages, forwardable_params(request), real_text))
}

fn forwardable_params(request: Object) -> Object {
    request
        .into_iter()
        .filter(|(name, value)| {
            !UNFORWARDED.contains(&name.as_str())
                && !matches!(value, Json::Null)
                && !matches!(value, Json::List(items) if items.is_empty())
        })
        .collect()
}

fn message_objects(request: &Object) -> Option<Vec<Object>> {
    let Json::List(items) = request.get(MESSAGES)? else {
        return None;
    };
    items
        .iter()
        .map(|item| match item {
            Json::Object(fields) => Some(fields.clone()),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
        })
        .collect()
}

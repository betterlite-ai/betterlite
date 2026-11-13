use betterlite_prompts::arguments::json::{Json, Object};

#[must_use]
pub fn request_is_streamed(request_data: &Object) -> bool {
    if matches!(request_data.get("stream"), Some(Json::Bool(true))) {
        return true;
    }
    let body = request_data
        .get("proxy_server_request")
        .and_then(Json::as_object)
        .and_then(|proxy| proxy.get("body"))
        .and_then(Json::as_object);
    matches!(
        body.and_then(|body_fields| body_fields.get("stream")),
        Some(Json::Bool(true))
    )
}

mod tests;

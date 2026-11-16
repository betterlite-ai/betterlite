use betterlite_prompts::arguments::json::{Json, Object};

// A header from the inbound request, when LiteLLM kept it on the request data.
#[must_use]
pub fn inbound_header(request_data: &Object, name: &str) -> Option<String> {
    let headers = request_data
        .get("proxy_server_request")
        .and_then(Json::as_object)?
        .get("headers")
        .and_then(Json::as_object)?;
    headers.iter().find_map(|(key, value)| {
        let text = value.as_str()?.trim();
        (key.eq_ignore_ascii_case(name) && !text.is_empty()).then(|| text.to_owned())
    })
}

mod tests;

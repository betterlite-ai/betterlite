use betterlite_config::sections::settings::JsonValue;
use betterlite_core::api::request_body::JsonObject;
use http::HeaderMap;

const PROVIDER_HEADER: &str = "custom-llm-provider";
const PROVIDER_FIELD: &str = "custom_llm_provider";

#[must_use]
pub fn provider_from_headers(headers: &HeaderMap) -> Option<String> {
    headers.get(PROVIDER_HEADER).map(|value| {
        value
            .as_bytes()
            .iter()
            .map(|byte| char::from(*byte))
            .collect()
    })
}

#[must_use]
pub fn provider_from_query(query: Option<&str>) -> Option<String> {
    let mut provider = None;
    for (name, value) in form_urlencoded::parse(query?.as_bytes()) {
        if name == PROVIDER_FIELD {
            provider = Some(value.into_owned());
        }
    }
    provider
}

#[must_use]
pub fn provider_from_body(body: &JsonObject) -> Option<&JsonValue> {
    // LiteLLM returns the value as it stands, so a null or a number comes back unchanged
    body.get(PROVIDER_FIELD)
}

mod tests;

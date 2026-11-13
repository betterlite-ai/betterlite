use betterlite_openai::api::speech_provider_contract::TextToSpeechRequestData;
use betterlite_openai::openai_base_config::Object;
use indexmap::IndexMap;
use serde_json::json;

#[must_use]
pub fn transform_request(
    model: &str,
    input: &str,
    optional_params: &Object,
) -> TextToSpeechRequestData {
    let mut params = optional_params.clone();
    let extra_body = params
        .remove("extra_body")
        .and_then(|extra| extra.as_object().cloned());
    let mut body = Object::new();
    body.insert("text".to_owned(), json!(input));
    body.insert("model_id".to_owned(), json!(model));
    body.extend(params.into_iter().filter(|(_, value)| !value.is_null()));
    if let Some(extra) = extra_body {
        body.extend(extra.into_iter().filter(|(_, value)| !value.is_null()));
    }
    let mut headers = IndexMap::new();
    headers.insert("Content-Type".to_owned(), "application/json".to_owned());
    TextToSpeechRequestData {
        dict_body: Some(body),
        ssml_body: None,
        headers: Some(headers),
    }
}

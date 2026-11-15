use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::openai_base_config::Object;

use super::options::{AUDIO_URL_FIELD, FILE_ID_FIELD, HandlerOptions};

pub const MODEL_FIELD: &str = "model";

#[must_use]
pub fn build_create_body(
    model: &str,
    params: &Object,
    options: &HandlerOptions,
    file_id: Option<&str>,
) -> Object {
    let mut body = Object::new();
    body.insert(MODEL_FIELD.to_owned(), JsonValue::Text(model.to_owned()));
    for (name, value) in params {
        if value.is_null() {
            continue;
        }
        body.insert(name.clone(), value.clone());
    }
    let audio_url = options.audio_url.as_deref().filter(|url| !url.is_empty());
    if let Some(url) = audio_url {
        body.insert(AUDIO_URL_FIELD.to_owned(), JsonValue::Text(url.to_owned()));
    }
    if let Some(selected) = file_id.filter(|value| !value.is_empty()) {
        body.insert(
            FILE_ID_FIELD.to_owned(),
            JsonValue::Text(selected.to_owned()),
        );
    }
    body
}

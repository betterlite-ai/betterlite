use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Object;
use indexmap::IndexMap;

use crate::api::passthrough_call::logging::json;

use super::mapping::{DEFAULT_ENGINE, DEFAULT_OUTPUT_FORMAT};
use super::voice::request_voice;

const SSML_TAG: &str = "<speak>";
const SSML_PREFIX: &str = "<speak ";
const OPTIONAL_FIELDS: [&str; 3] = ["LanguageCode", "LexiconNames", "SampleRate"];

// LiteLLM's request body: five fixed fields in this order, then the present Polly fields including nulls
#[must_use]
pub fn request_body(
    input: &str,
    voice: Option<&str>,
    optional_params: &Object,
) -> IndexMap<String, Json> {
    let text_type = if input.contains(SSML_TAG) || input.contains(SSML_PREFIX) {
        "ssml"
    } else {
        "text"
    };
    let mut body = IndexMap::new();
    body.insert(
        String::from("Engine"),
        configured(optional_params, "engine", DEFAULT_ENGINE),
    );
    body.insert(
        String::from("OutputFormat"),
        configured(optional_params, "output_format", DEFAULT_OUTPUT_FORMAT),
    );
    body.insert(String::from("Text"), Json::Text(input.to_owned()));
    body.insert(
        String::from("VoiceId"),
        Json::Text(request_voice(voice).to_owned()),
    );
    body.insert(String::from("TextType"), Json::Text(text_type.to_owned()));
    for name in OPTIONAL_FIELDS {
        if let Some(value) = optional_params.get(name) {
            body.insert(name.to_owned(), json(value));
        }
    }
    body
}

fn configured(optional_params: &Object, name: &str, default: &str) -> Json {
    optional_params
        .get(name)
        .filter(|value| !value.is_null())
        .map_or_else(|| Json::Text(default.to_owned()), json)
}

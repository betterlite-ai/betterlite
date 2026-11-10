use betterlite_openai::chat_content::{ChatContent, Mode};
use betterlite_prompts::arguments::json::{Json, Object};

use super::error::GemmaEnvelopeError;

pub const REQUEST_FORMAT_KEY: &str = "@requestFormat";
pub const CHAT_COMPLETIONS: &str = "chatCompletions";

pub fn transform_request(content: &ChatContent<'_>) -> Result<Json, GemmaEnvelopeError> {
    let shaped = content.transform(Mode::Sync)?;
    let mut body = shaped.to_body()?;
    body.shift_remove("model");
    body.shift_remove("stream");
    body.shift_remove("stream_options");
    body.shift_remove("context_management");
    let mut instance = Object::new();
    instance.insert(
        REQUEST_FORMAT_KEY.to_owned(),
        Json::Text(CHAT_COMPLETIONS.to_owned()),
    );
    instance.extend(body);
    Ok(Json::Object(Object::from_iter([(
        String::from("instances"),
        Json::List(vec![Json::Object(instance)]),
    )])))
}

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::openai_dispatch::logging::LoggedCall;

// main.py's logging.post_call, as one port the composition writes and a fixture or the proxy reads back
pub trait FoundryLogger: Send {
    fn post_call(&mut self, record: LoggedCall);
}

// A buffered reply's model and first message text, the two fields both sides read off ModelResponse
#[must_use]
pub fn reply_report(model: Option<&str>, content: Option<&str>) -> Json {
    Json::Object(IndexMap::from([
        (
            String::from("model"),
            model.map_or(Json::Null, |text| Json::Text(text.to_owned())),
        ),
        (
            String::from("content"),
            content.map_or(Json::Null, |text| Json::Text(text.to_owned())),
        ),
    ]))
}

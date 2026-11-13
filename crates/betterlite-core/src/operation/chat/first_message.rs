use indexmap::IndexMap;
use serde::Deserialize;
use serde::de::IgnoredAny;

mod tests;

#[derive(Deserialize)]
struct Body {
    #[serde(default)]
    messages: Vec<Message>,
}

// the typed message folds an absent content key and a null one together, so the keys are read as they came
#[derive(Deserialize)]
struct Message {
    #[serde(flatten)]
    keys: IndexMap<String, IgnoredAny>,
}

// LiteLLM runs the pre-call rules only when the first message has a content key, null included
#[must_use]
pub fn first_message_has_content(body: &[u8]) -> bool {
    serde_json::from_slice::<Body>(body).is_ok_and(|parsed| {
        parsed
            .messages
            .first()
            .is_some_and(|first| first.keys.contains_key("content"))
    })
}

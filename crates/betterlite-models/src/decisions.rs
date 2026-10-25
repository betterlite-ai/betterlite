pub mod types;

use types::DecisionsJson;

#[must_use]
pub fn decisions_text(value: &DecisionsJson) -> String {
    match value {
        DecisionsJson::String(text) => text.clone(),
        DecisionsJson::Null
        | DecisionsJson::Bool(_)
        | DecisionsJson::Number(_)
        | DecisionsJson::Array(_)
        | DecisionsJson::Object(_) => serde_json::to_string(value).unwrap_or_default(),
    }
}

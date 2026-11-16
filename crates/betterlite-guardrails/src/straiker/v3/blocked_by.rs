use betterlite_prompts::arguments::json::{Json, Object};

// The block-mode subset of the fired controls, sorted so the order is the same on every run. Every list item is a control: a string stands for itself and anything else for its compact JSON text, so it still blocks the turn.
#[must_use]
pub fn blocking_controls(verdict: &Object) -> Vec<String> {
    let Some(Json::List(raw)) = verdict.get("blocked_by") else {
        return Vec::new();
    };
    let mut controls: Vec<String> = raw
        .iter()
        .map(|item| match item {
            Json::Text(text) => text.clone(),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                serde_json::to_string(item).unwrap_or_default()
            }
        })
        .collect();
    controls.sort();
    controls
}

mod tests;

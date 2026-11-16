use betterlite_prompts::arguments::json::{Json, Object};

// The block-mode subset of the fired controls, sorted so the order is the same on every run.
#[must_use]
pub fn blocking_controls(verdict: &Object) -> Vec<String> {
    let Some(Json::List(raw)) = verdict.get("blocked_by") else {
        return Vec::new();
    };
    let mut controls: Vec<String> = raw
        .iter()
        .filter_map(Json::as_str)
        .map(str::to_owned)
        .collect();
    controls.sort();
    controls
}

mod tests;

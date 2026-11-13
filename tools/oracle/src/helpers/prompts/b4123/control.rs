use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::cache_accounting::control::default_control;
use serde_json::{Value, json};

pub(crate) fn controls(settings: &[Option<String>]) -> Value {
    let controls: Vec<Json> = settings
        .iter()
        .map(|setting| default_control(setting.as_deref()))
        .collect();
    json!({ "controls": controls })
}

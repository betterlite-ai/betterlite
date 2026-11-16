use betterlite_prompts::arguments::json::{Json, Object};
use serde_json::Number;

const INPUT_DEFAULTS: [&str; 3] = ["cached_tokens", "text_tokens", "audio_tokens"];
const OUTPUT_DEFAULTS: [&str; 2] = ["text_tokens", "audio_tokens"];

// Pipecat reads the singular names, so each is filled from the plural details unless the usage already carries one
pub fn add_usage_detail_aliases(usage: &mut Object) {
    add_alias(
        usage,
        "input_token_details",
        "input_tokens_details",
        &INPUT_DEFAULTS,
    );
    add_alias(
        usage,
        "output_token_details",
        "output_tokens_details",
        &OUTPUT_DEFAULTS,
    );
}

fn add_alias(usage: &mut Object, alias: &str, plural: &str, defaults: &[&str]) {
    if usage.contains_key(alias) {
        return;
    }
    let mut details: Object = defaults
        .iter()
        .map(|name| ((*name).to_owned(), Json::Number(Number::from(0_i64))))
        .collect();
    if let Some(Json::Object(reported)) = usage.get(plural) {
        for (name, value) in reported {
            if *value != Json::Null {
                details.insert(name.clone(), value.clone());
            }
        }
    }
    usage.insert(alias.to_owned(), Json::Object(details));
}

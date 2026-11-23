use betterlite_prompts::arguments::json::Json;
use serde_json::value::to_raw_value;

use crate::invoke_primitives::config::Params;

use super::prompt::{PromptError, convert_messages_to_prompt};

// AmazonQwen3Config.get_supported_openai_params returns exactly these names, whatever the model is.
pub const SUPPORTED_OPENAI_PARAMS: [&str; 6] = [
    "max_tokens",
    "temperature",
    "top_p",
    "top_k",
    "stop",
    "stream",
];

// AmazonQwen3Config's constructor fields; an absent value is the class default the constructor leaves unset.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Options {
    pub max_tokens: Option<i64>,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub top_k: Option<i64>,
    pub stop: Option<Vec<String>>,
}

// AmazonQwen3Config.map_openai_params copies each supported name unchanged and drops every other name.
#[must_use]
pub fn map_openai_params(non_default_params: &Params, mut optional_params: Params) -> Params {
    for (name, value) in non_default_params {
        if SUPPORTED_OPENAI_PARAMS.contains(&name.as_str()) {
            optional_params.insert(name.clone(), value.clone());
        }
    }
    optional_params
}

// AmazonQwen3Config.transform_request: the prompt, then only the supplied parameters, with max_tokens renamed.
pub fn transform_request(
    messages: &[Json],
    optional_params: &Params,
) -> Result<Params, PromptError> {
    let prompt = convert_messages_to_prompt(messages)?;
    let mut body = Params::new();
    body.insert(
        String::from("prompt"),
        to_raw_value(&prompt).map_err(PromptError::Encode)?,
    );
    copy_param(optional_params, &mut body, "max_tokens", "max_gen_len");
    copy_param(optional_params, &mut body, "temperature", "temperature");
    copy_param(optional_params, &mut body, "top_p", "top_p");
    copy_param(optional_params, &mut body, "top_k", "top_k");
    copy_param(optional_params, &mut body, "stop", "stop");
    Ok(body)
}

fn copy_param(source: &Params, target: &mut Params, name: &str, projected: &str) {
    if let Some(value) = source.get(name) {
        target.insert(projected.to_owned(), value.clone());
    }
}

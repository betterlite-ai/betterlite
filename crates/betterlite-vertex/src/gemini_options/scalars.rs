use betterlite_prompts::arguments::json::{Json, Object};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::gemini_thinking::audio::map_response_modalities;
use crate::gemini_tools::options::map_service_tier;

use super::convert;
use super::error::GeminiOptionsError;
use super::generation;
use super::params::supports_penalty_parameters;

// LiteLLM accepts a single string or a list for stop
#[derive(Deserialize)]
#[serde(untagged)]
enum Stop {
    Single(String),
    Many(Vec<String>),
}

pub fn map(
    param: &str,
    value: &Json,
    optional: &mut Object,
    model: &str,
    warned: &mut bool,
) -> Result<(), GeminiOptionsError> {
    match param {
        "temperature" | "top_p" | "top_k" => {
            generation::check_sampling(model, param, value, warned)?;
            optional.insert(param.to_owned(), value.clone());
        }
        "n" => insert::<i64>(optional, "candidate_count", param, value)?,
        "max_tokens" | "max_completion_tokens" => {
            insert::<i64>(optional, "max_output_tokens", param, value)?;
        }
        "seed" => insert::<i64>(optional, "seed", param, value)?,
        "top_logprobs" => insert::<i64>(optional, "logprobs", param, value)?,
        "logprobs" => insert::<bool>(optional, "responseLogprobs", param, value)?,
        "stream" => map_stream(value, optional)?,
        "stop" => map_stop(value, optional)?,
        "modalities" => {
            let requested: Vec<String> = convert::decode(value, param)?;
            let modalities = map_response_modalities(&requested);
            optional.insert(
                "responseModalities".to_owned(),
                convert::encode(modalities)?,
            );
        }
        "service_tier" => {
            let tier: String = convert::decode(value, param)?;
            optional.insert(
                "service_tier".to_owned(),
                Json::Text(map_service_tier(&tier)),
            );
        }
        "frequency_penalty" | "presence_penalty" => {
            convert::decode::<f64>(value, param)?;
            if supports_penalty_parameters(model) {
                optional.insert(param.to_owned(), value.clone());
            }
        }
        _ => {}
    }
    Ok(())
}

fn insert<T>(
    optional: &mut Object,
    target: &str,
    param: &str,
    value: &Json,
) -> Result<(), GeminiOptionsError>
where
    T: DeserializeOwned,
{
    convert::decode::<T>(value, param)?;
    optional.insert(target.to_owned(), value.clone());
    Ok(())
}

fn map_stream(value: &Json, optional: &mut Object) -> Result<(), GeminiOptionsError> {
    if convert::decode::<bool>(value, "stream")? {
        optional.insert("stream".to_owned(), Json::Bool(true));
    }
    Ok(())
}

// LiteLLM never sends stream=false and always wraps a string stop in a list
fn map_stop(value: &Json, optional: &mut Object) -> Result<(), GeminiOptionsError> {
    let sequences = match convert::decode::<Stop>(value, "stop")? {
        Stop::Single(text) => vec![Json::Text(text)],
        Stop::Many(items) => items.into_iter().map(Json::Text).collect(),
    };
    optional.insert("stop_sequences".to_owned(), Json::List(sequences));
    Ok(())
}

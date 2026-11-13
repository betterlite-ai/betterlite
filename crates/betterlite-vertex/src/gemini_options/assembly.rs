use betterlite_config::sections::vertex_schema_shaping::Settings as SchemaSettings;
use betterlite_prompts::arguments::json::{Json, Object};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Number;

use crate::gemini_format::response::apply_response_format;
use crate::gemini_thinking::audio::{
    AudioParam, ResponseModality, map_audio_params, map_response_modalities,
};
use crate::gemini_thinking::server_flags::apply_server_tool_flags;
use crate::gemini_thinking::settings::{MinimalEffortSupport, ThinkingSettings};
use crate::gemini_tools::options::map_service_tier;
use crate::model_urls::model_name::is_gemini_3_or_newer;

use super::defaults::OptionSettings;
use super::error::{GeminiOptionsError, decode, encode};
use super::params::supports_penalty_parameters;
use super::thinking;
use super::tools;

// LiteLLM accepts a single string or a list for stop
#[derive(Deserialize)]
#[serde(untagged)]
enum Stop {
    Single(String),
    Many(Vec<String>),
}

pub struct OptionsContext<'request> {
    pub model: &'request str,
    pub non_default: &'request Object,
    pub settings: &'request OptionSettings,
    pub schema: &'request SchemaSettings,
    pub thinking: &'request ThinkingSettings,
    pub support: &'request dyn MinimalEffortSupport,
}

// Returns how many warnings LiteLLM logs for the request, so a caller can report them
pub fn map_options(
    non_default: &Object,
    optional: &mut Object,
    context: &OptionsContext<'_>,
) -> Result<usize, GeminiOptionsError> {
    apply_server_tool_flags(non_default, optional);
    let mut warned = false;
    let mut warnings = 0_usize;
    for (param, value) in non_default {
        if !matches!(value, Json::Null) {
            warnings =
                warnings.saturating_add(map_param(param, value, optional, context, &mut warned)?);
        }
    }
    if let Some(safety) = &context.settings.safety_settings {
        optional.insert("safety_settings".to_owned(), safety.clone());
    }
    ensure_audio_modality(optional)?;
    if is_gemini_3_or_newer(context.model) && !optional.contains_key("temperature") {
        let temperature =
            Number::from_f64(1.0_f64).ok_or_else(|| GeminiOptionsError::NonFinite {
                field: "temperature".to_owned(),
            })?;
        optional.insert("temperature".to_owned(), Json::Number(temperature));
    }
    warnings = warnings.saturating_add(usize::from(tools::drop_mixed_search(optional)));
    Ok(warnings)
}

fn map_param(
    param: &str,
    value: &Json,
    optional: &mut Object,
    context: &OptionsContext<'_>,
    warned: &mut bool,
) -> Result<usize, GeminiOptionsError> {
    if param == "tools" || param == "functions" {
        return tools::append(value, optional, context.schema);
    }
    if matches!(
        param,
        "tool_choice" | "parallel_tool_calls" | "web_search_options"
    ) {
        tools::map(param, value, optional, context)?;
    } else if matches!(param, "audio" | "response_format") {
        map_media_param(param, value, optional, context)?;
    } else if matches!(param, "reasoning_effort" | "thinking") {
        thinking::map(param, value, optional, context)?;
    } else if param == "include_server_side_tool_invocations" {
        if decode::<bool>(value, param)? {
            optional.insert(param.to_owned(), Json::Bool(true));
        }
    } else {
        map_scalar(param, value, optional, context.model, warned)?;
    }
    Ok(0)
}

fn map_media_param(
    param: &str,
    value: &Json,
    optional: &mut Object,
    context: &OptionsContext<'_>,
) -> Result<(), GeminiOptionsError> {
    match param {
        "response_format" => {
            let format: Object = decode(value, param)?;
            apply_response_format(&format, context.model, optional, context.schema)?;
        }
        _ => {
            let audio: AudioParam = decode(value, param)?;
            let speech = map_audio_params(&audio)?;
            optional.insert("speechConfig".to_owned(), Json::Object(speech));
        }
    }
    Ok(())
}

fn map_scalar(
    param: &str,
    value: &Json,
    optional: &mut Object,
    model: &str,
    warned: &mut bool,
) -> Result<(), GeminiOptionsError> {
    match param {
        "temperature" | "top_p" | "top_k" => {
            check_sampling(model, param, value, warned)?;
            optional.insert(param.to_owned(), value.clone());
        }
        "n" => insert::<i64>(optional, "candidate_count", param, value)?,
        "max_tokens" | "max_completion_tokens" => {
            insert::<i64>(optional, "max_output_tokens", param, value)?;
        }
        "seed" => insert::<i64>(optional, "seed", param, value)?,
        "top_logprobs" => insert::<i64>(optional, "logprobs", param, value)?,
        "logprobs" => insert::<bool>(optional, "responseLogprobs", param, value)?,
        "stream" => {
            if decode::<bool>(value, param)? {
                optional.insert("stream".to_owned(), Json::Bool(true));
            }
        }
        "stop" => {
            let sequences = match decode::<Stop>(value, param)? {
                Stop::Single(text) => vec![Json::Text(text)],
                Stop::Many(items) => items.into_iter().map(Json::Text).collect(),
            };
            optional.insert("stop_sequences".to_owned(), Json::List(sequences));
        }
        "modalities" => {
            let requested: Vec<String> = decode(value, param)?;
            let modalities = map_response_modalities(&requested);
            optional.insert("responseModalities".to_owned(), encode(modalities)?);
        }
        "service_tier" => {
            let tier: String = decode(value, param)?;
            optional.insert(
                "service_tier".to_owned(),
                Json::Text(map_service_tier(&tier)),
            );
        }
        "frequency_penalty" | "presence_penalty" => {
            decode::<f64>(value, param)?;
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
    decode::<T>(value, param)?;
    optional.insert(target.to_owned(), value.clone());
    Ok(())
}

// Gemini 3 keeps sampling but warns once, on the first of the three it maps
fn check_sampling(
    model: &str,
    param: &str,
    value: &Json,
    warned: &mut bool,
) -> Result<(), GeminiOptionsError> {
    let Json::Number(number) = value else {
        return Err(GeminiOptionsError::FieldType {
            param: param.to_owned(),
            expected: "a number",
        });
    };
    let Some(parsed) = number.as_f64() else {
        return Err(GeminiOptionsError::NonFinite {
            field: param.to_owned(),
        });
    };
    if !is_gemini_3_or_newer(model) {
        return Ok(());
    }
    if param == "temperature" && parsed < 1.0_f64 {
        tracing::info!(
            model,
            "temperature below 1.0 for Gemini 3 can cause loops and weaker reasoning; 1.0 is recommended"
        );
    }
    if !*warned {
        tracing::warn!(
            model,
            "temperature, top_p and top_k still work for Gemini 3 but are planned for removal; put sampling guidance in the system instructions instead"
        );
        *warned = true;
    }
    Ok(())
}

// LiteLLM adds AUDIO to responseModalities whenever a speechConfig is present
fn ensure_audio_modality(optional: &mut Object) -> Result<(), GeminiOptionsError> {
    if !optional.contains_key("speechConfig") {
        return Ok(());
    }
    let audio = encode(ResponseModality::Audio)?;
    if !optional.contains_key("responseModalities") {
        optional.insert("responseModalities".to_owned(), Json::List(vec![audio]));
        return Ok(());
    }
    let Some(Json::List(modalities)) = optional.get_mut("responseModalities") else {
        return Err(GeminiOptionsError::NotList {
            param: "responseModalities".to_owned(),
        });
    };
    if !modalities.contains(&audio) {
        modalities.push(audio);
    }
    Ok(())
}

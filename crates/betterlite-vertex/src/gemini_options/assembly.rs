use betterlite_config::sections::vertex_schema_shaping::Settings as SchemaSettings;
use betterlite_prompts::arguments::json::{Json, Object};
use serde::Deserialize;
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
    map_group(param, value, optional, context)?;
    map_scalar(param, value, optional, context.model, warned)
}

fn map_group(
    param: &str,
    value: &Json,
    optional: &mut Object,
    context: &OptionsContext<'_>,
) -> Result<(), GeminiOptionsError> {
    if matches!(
        param,
        "tool_choice" | "parallel_tool_calls" | "web_search_options"
    ) {
        return tools::map(param, value, optional, context);
    }
    if matches!(param, "reasoning_effort" | "thinking") {
        return thinking::map(param, value, optional, context);
    }
    if param == "response_format" {
        let format: Object = decode(value, param)?;
        return apply_response_format(&format, context.model, optional, context.schema)
            .map_err(GeminiOptionsError::Format);
    }
    if param == "audio" {
        let audio: AudioParam = decode(value, param)?;
        let speech = map_audio_params(&audio)?;
        optional.insert("speechConfig".to_owned(), Json::Object(speech));
        return Ok(());
    }
    if param == "include_server_side_tool_invocations" && decode::<bool>(value, param)? {
        optional.insert(param.to_owned(), Json::Bool(true));
    }
    Ok(())
}

// LiteLLM renames these OpenAI names onto their Gemini fields and validates the JSON type
const SCALARS: [(&str, &str, ScalarKind); 6] = [
    ("n", "candidate_count", ScalarKind::Integer),
    ("max_tokens", "max_output_tokens", ScalarKind::Integer),
    (
        "max_completion_tokens",
        "max_output_tokens",
        ScalarKind::Integer,
    ),
    ("seed", "seed", ScalarKind::Integer),
    ("top_logprobs", "logprobs", ScalarKind::Integer),
    ("logprobs", "responseLogprobs", ScalarKind::Boolean),
];

#[derive(Clone, Copy)]
enum ScalarKind {
    Integer,
    Boolean,
}

fn map_scalar(
    param: &str,
    value: &Json,
    optional: &mut Object,
    model: &str,
    warned: &mut bool,
) -> Result<usize, GeminiOptionsError> {
    if matches!(param, "temperature" | "top_p" | "top_k") {
        let deprecation = check_sampling(model, param, value, warned)?;
        optional.insert(param.to_owned(), value.clone());
        return Ok(usize::from(deprecation));
    }
    if matches!(param, "frequency_penalty" | "presence_penalty") {
        decode::<f64>(value, param)?;
        if supports_penalty_parameters(model) {
            optional.insert(param.to_owned(), value.clone());
        }
        return Ok(0);
    }
    for (name, target, kind) in SCALARS {
        if name != param {
            continue;
        }
        match kind {
            ScalarKind::Integer => {
                decode::<i64>(value, param)?;
            }
            ScalarKind::Boolean => {
                decode::<bool>(value, param)?;
            }
        }
        optional.insert(target.to_owned(), value.clone());
        return Ok(0);
    }
    match param {
        "stop" => map_stop(value, optional)?,
        "modalities" => map_modalities(value, optional)?,
        "service_tier" => map_tier(value, optional)?,
        "stream" => map_stream(value, optional)?,
        _ => {}
    }
    Ok(0)
}

fn map_stop(value: &Json, optional: &mut Object) -> Result<(), GeminiOptionsError> {
    let sequences = match decode::<Stop>(value, "stop")? {
        Stop::Single(text) => vec![Json::Text(text)],
        Stop::Many(items) => items.into_iter().map(Json::Text).collect(),
    };
    optional.insert("stop_sequences".to_owned(), Json::List(sequences));
    Ok(())
}

fn map_modalities(value: &Json, optional: &mut Object) -> Result<(), GeminiOptionsError> {
    let requested: Vec<String> = decode(value, "modalities")?;
    let modalities = map_response_modalities(&requested);
    optional.insert("responseModalities".to_owned(), encode(modalities)?);
    Ok(())
}

fn map_tier(value: &Json, optional: &mut Object) -> Result<(), GeminiOptionsError> {
    let tier: String = decode(value, "service_tier")?;
    let mapped = Json::Text(map_service_tier(&tier));
    optional.insert("service_tier".to_owned(), mapped);
    Ok(())
}

fn map_stream(value: &Json, optional: &mut Object) -> Result<(), GeminiOptionsError> {
    if decode::<bool>(value, "stream")? {
        optional.insert("stream".to_owned(), Json::Bool(true));
    }
    Ok(())
}

// Gemini 3 keeps sampling but warns once, on the first of the three it maps
fn check_sampling(
    model: &str,
    param: &str,
    value: &Json,
    warned: &mut bool,
) -> Result<bool, GeminiOptionsError> {
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
        return Ok(false);
    }
    if param == "temperature" && parsed < 1.0_f64 {
        tracing::info!(
            model,
            "temperature below 1.0 for Gemini 3 can cause loops and weaker reasoning; 1.0 is recommended"
        );
    }
    if *warned {
        return Ok(false);
    }
    tracing::warn!(
        model,
        "temperature, top_p and top_k still work for Gemini 3 but are planned for removal; put sampling guidance in the system instructions instead"
    );
    *warned = true;
    Ok(true)
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

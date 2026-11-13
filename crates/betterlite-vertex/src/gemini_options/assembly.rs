use betterlite_config::sections::vertex_schema_shaping::Settings as SchemaSettings;
use betterlite_prompts::arguments::json::{Json, Object};
use serde_json::Number;

use crate::gemini_format::response::apply_response_format;
use crate::gemini_thinking::server_flags::apply_server_tool_flags;
use crate::gemini_thinking::settings::{MinimalEffortSupport, ThinkingSettings};
use crate::model_urls::model_name::is_gemini_3_or_newer;

use super::defaults::OptionSettings;
use super::error::GeminiOptionsError;
use super::media;
use super::scalars;
use super::thinking;
use super::tools;

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
        warnings =
            warnings.saturating_add(map_param(param, value, optional, context, &mut warned)?);
    }
    if let Some(safety) = &context.settings.safety_settings {
        optional.insert("safety_settings".to_owned(), safety.clone());
    }
    media::ensure_audio_modality(optional)?;
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
    match param {
        "audio" => media::map_audio(value, optional)?,
        "modalities" => media::map_modalities(value, optional)?,
        "response_format" => map_response_format(value, optional, context)?,
        "reasoning_effort" => thinking::map_reasoning_effort(value, optional, context)?,
        "thinking" => thinking::map_thinking(value, optional, context)?,
        "tools" | "functions" => {
            return tools::append(value, optional, context.schema);
        }
        "tool_choice" => tools::map_choice(value, optional)?,
        "parallel_tool_calls" => {
            tools::map_parallel(value, context.non_default, optional)?;
        }
        "web_search_options" => tools::map_web_search(value, optional)?,
        "include_server_side_tool_invocations" => map_server_flag(value, optional)?,
        _ => {
            scalars::map(param, value, optional, context.model, warned)?;
        }
    }
    Ok(0)
}

fn map_response_format(
    value: &Json,
    optional: &mut Object,
    context: &OptionsContext<'_>,
) -> Result<(), GeminiOptionsError> {
    match value {
        Json::Object(format) => {
            apply_response_format(format, context.model, optional, context.schema)?;
            Ok(())
        }
        Json::Null => Ok(()),
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Err(GeminiOptionsError::FieldType {
                param: "response_format".to_owned(),
                expected: "an object",
            })
        }
    }
}

fn map_server_flag(value: &Json, optional: &mut Object) -> Result<(), GeminiOptionsError> {
    match value {
        Json::Bool(true) => {
            optional.insert(
                "include_server_side_tool_invocations".to_owned(),
                Json::Bool(true),
            );
            Ok(())
        }
        Json::Bool(false) | Json::Null => Ok(()),
        Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => {
            Err(GeminiOptionsError::FieldType {
                param: "include_server_side_tool_invocations".to_owned(),
                expected: "a boolean",
            })
        }
    }
}

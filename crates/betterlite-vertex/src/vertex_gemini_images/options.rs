use betterlite_config::sections::vertex_schema_shaping::Settings;
use betterlite_prompts::arguments::json::{Json, Object};

use super::error::GeminiImageError;
use crate::gemini_image_policy::tools::map_gemini_image_tools_params;
use crate::vertex_image_identity::size_aspect_ratio;

// get_supported_openai_params: both native spellings and the OpenAI n/size pair
pub const SUPPORTED_OPENAI_PARAMS: [&str; 9] = [
    "n",
    "size",
    "imageConfig",
    "aspectRatio",
    "aspect_ratio",
    "imageSize",
    "image_size",
    "tools",
    "web_search_options",
];

#[derive(Debug, Default, PartialEq, Eq)]
pub struct MappedOptions {
    pub params: Object,
    // LiteLLM logs one warning for every non-object imageConfig, a warning we count for the caller
    pub warnings: usize,
}

pub fn map_openai_params(
    non_default_params: &Object,
    optional_params: &Object,
    settings: &Settings,
) -> Result<MappedOptions, GeminiImageError> {
    let mut mapped = Object::new();
    let mut warnings = 0_usize;
    for (key, value) in non_default_params {
        if optional_params.contains_key(key) || !SUPPORTED_OPENAI_PARAMS.contains(&key.as_str()) {
            continue;
        }
        match key.as_str() {
            "n" => {
                mapped.insert("candidate_count".to_owned(), value.clone());
            }
            "size" => {
                let Json::Text(size) = value else {
                    return Err(GeminiImageError::FieldType {
                        param: "size",
                        expected: "a string",
                    });
                };
                let ratio = size_aspect_ratio(size);
                mapped.insert("aspectRatio".to_owned(), Json::Text(ratio.to_owned()));
            }
            "aspectRatio" | "aspect_ratio" => {
                mapped.insert("aspectRatio".to_owned(), value.clone());
            }
            "imageSize" | "image_size" => {
                mapped.insert("imageSize".to_owned(), value.clone());
            }
            "imageConfig" => match value {
                Json::Object(config) => {
                    mapped.insert("imageConfig".to_owned(), Json::Object(config.clone()));
                }
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
                    warnings = warnings.saturating_add(1);
                    tracing::warn!(
                        "imageConfig must be a dict, got {} — ignoring.",
                        python_type(value)
                    );
                }
            },
            // tools and web_search_options belong to the tools mapping below, which reads them itself
            _ => {}
        }
    }
    let params = map_gemini_image_tools_params(non_default_params, &mapped, settings)
        .map_err(GeminiImageError::Policy)?;
    Ok(MappedOptions { params, warnings })
}

// %s in LiteLLM's warning is type(v).__name__
fn python_type(value: &Json) -> &'static str {
    match value {
        Json::Null => "NoneType",
        Json::Bool(_) => "bool",
        Json::Number(number) if number.is_i64() || number.is_u64() => "int",
        Json::Number(_) => "float",
        Json::Text(_) => "str",
        Json::List(_) => "list",
        Json::Object(_) => "dict",
    }
}

use betterlite_config::sections::vertex_schema_shaping::Settings as SchemaSettings;
use betterlite_prompts::arguments::json::Object;

use super::error::StudioImageError;
use crate::gemini_image_policy::policy::{
    ImageParamMapping, is_gemini_image_model, map_image_params,
};
use crate::gemini_image_policy::tools::map_gemini_image_tools_params;

// get_supported_openai_params: Imagen reads n and size, and a gemini image model adds its own three
#[must_use]
pub fn supported_openai_params(model: &str) -> Vec<&'static str> {
    let mut supported = Vec::from(["n", "size"]);
    if is_gemini_image_model(model) {
        supported.extend(["imageConfig", "tools", "web_search_options"]);
    }
    supported
}

// map_openai_params: B1334's map_openai_image_params_to_gemini, then the tools and search mapping
pub fn map_openai_params(
    non_default_params: &Object,
    optional_params: &Object,
    model: &str,
    image_size_support: Option<bool>,
) -> Result<Object, StudioImageError> {
    let supported = supported_openai_params(model);
    let mapped = map_image_params(&ImageParamMapping {
        params: non_default_params,
        model,
        supported_params: &supported,
        optional_params,
        image_size_support,
        parse_image_config_string: false,
    })?;
    if !is_gemini_image_model(model) {
        return Ok(mapped);
    }
    // LiteLLM's image tools mapping builds a fresh VertexGeminiConfig, so the schema settings stay at their defaults
    map_gemini_image_tools_params(non_default_params, &mapped, &SchemaSettings::default())
        .map_err(StudioImageError::Policy)
}

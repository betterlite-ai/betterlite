use serde_json::value::RawValue;
use serde_json::value::to_raw_value;
use std::collections::{BTreeMap, BTreeSet};

use crate::entry::ModelFields;
use crate::price_fields::PriceFields;

const FIXED_FIELDS: &str = "key max_tokens max_input_tokens max_output_tokens input_cost_per_token input_cost_per_token_flex input_cost_per_token_priority input_cost_per_token_balanced input_cost_per_token_ultrafast cache_creation_input_token_cost cache_creation_input_token_cost_above_200k_tokens cache_creation_input_token_cost_above_272k_tokens cache_creation_input_token_cost_above_272k_tokens_priority cache_creation_input_token_cost_above_272k_tokens_flex cache_creation_input_token_cost_above_272k_tokens_ultrafast cache_creation_input_token_cost_flex cache_creation_input_token_cost_priority cache_creation_input_token_cost_ultrafast cache_read_input_token_cost cache_read_input_audio_token_cost prompt_cache_min_tokens cache_read_input_token_cost_above_200k_tokens cache_read_input_token_cost_above_200k_tokens_priority cache_read_input_token_cost_above_272k_tokens cache_read_input_token_cost_above_272k_tokens_priority cache_read_input_token_cost_above_272k_tokens_flex cache_read_input_token_cost_above_272k_tokens_ultrafast cache_read_input_token_cost_above_512k_tokens cache_read_input_token_cost_flex cache_read_input_token_cost_priority cache_read_input_token_cost_balanced cache_read_input_token_cost_ultrafast cache_read_input_token_cost_batches cache_read_input_token_cost_above_200k_tokens_batches cache_read_input_token_cost_above_272k_tokens_batches cache_creation_input_token_cost_batches cache_creation_input_token_cost_above_200k_tokens_batches cache_creation_input_token_cost_above_272k_tokens_batches cache_creation_input_token_cost_above_1hr off_peak_pricing input_cost_per_character input_cost_per_token_above_128k_tokens input_cost_per_token_above_200k_tokens input_cost_per_token_above_200k_tokens_priority input_cost_per_token_above_272k_tokens input_cost_per_token_above_272k_tokens_priority input_cost_per_token_above_272k_tokens_flex input_cost_per_token_above_272k_tokens_ultrafast input_cost_per_token_above_512k_tokens input_cost_per_query cost_per_second input_cost_per_second input_cost_per_audio_token input_cost_per_image_token input_cost_per_video_token input_cost_per_audio_token_batches input_cost_per_image_token_batches input_cost_per_image input_cost_per_audio_per_second input_cost_per_video_per_second input_cost_per_token_batches input_cost_per_video_token_batches input_cost_per_token_above_200k_tokens_batches input_cost_per_token_above_272k_tokens_batches output_cost_per_token_batches output_cost_per_token_above_200k_tokens_batches output_cost_per_token_above_272k_tokens_batches output_cost_per_token output_cost_per_token_flex output_cost_per_token_priority output_cost_per_token_balanced output_cost_per_token_ultrafast regional_processing_uplift_multiplier_eu regional_processing_uplift_multiplier_us regional_endpoint_uplift_multiplier output_cost_per_audio_token output_cost_per_character output_cost_per_reasoning_token output_cost_per_reasoning_token_flex output_cost_per_reasoning_token_priority output_cost_per_token_above_128k_tokens output_cost_per_character_above_128k_tokens output_cost_per_token_above_200k_tokens output_cost_per_token_above_200k_tokens_priority output_cost_per_token_above_272k_tokens output_cost_per_token_above_272k_tokens_priority output_cost_per_token_above_272k_tokens_flex output_cost_per_token_above_272k_tokens_ultrafast output_cost_per_token_above_512k_tokens output_cost_per_second output_cost_per_second_1080p output_cost_per_second_480p output_cost_per_second_720p output_cost_per_second_768p output_cost_per_second_2k output_cost_per_second_4k output_cost_per_video_per_second output_cost_per_image output_cost_per_pixel output_cost_per_image_token output_cost_per_video_token output_vector_size citation_cost_per_token tiered_pricing litellm_provider mode supported_endpoints supports_system_messages supports_response_schema supports_vision supports_function_calling supports_parallel_function_calling supports_tool_choice supports_assistant_prefill supports_prompt_caching supports_prompt_cache_breakpoint supports_thinking_cache_preservation supports_audio_input supports_audio_output supports_pdf_input supports_embedding_image_input supports_native_streaming supports_native_structured_output supports_web_search supports_url_context supports_reasoning supports_adaptive_thinking supports_legacy_thinking thinking_always_on supports_tool_search supports_mid_conversation_system supports_anthropic_thinking_payload supports_anthropic_compaction supports_none_reasoning_effort supports_minimal_reasoning_effort supports_low_reasoning_effort supports_xhigh_reasoning_effort supports_max_reasoning_effort reasoning_effort_levels default_reasoning_effort bedrock_output_config_effort_ceiling bedrock_converse_supports_strict_tools supports_computer_use search_context_cost_per_query web_search_billing_unit google_maps_grounding_cost_per_query tpm rpm ocr_cost_per_page ocr_cost_per_page_batches ocr_cost_per_credit annotation_cost_per_page annotation_cost_per_page_batches provider_specific_entry uses_embed_content supports_image_size supported_audio_formats vertex_ai_audio_api";

#[derive(Debug, thiserror::Error)]
pub enum ProjectionError {
    #[error("model metadata must be an object: {0}")]
    Object(#[from] serde_json::Error),
    #[error("model metadata fields: {0}")]
    Fields(String),
    #[error("model metadata prices: {0}")]
    Prices(String),
}

pub fn project(
    key: &str,
    _model: &str,
    provider: &str,
    entry: &RawValue,
) -> Result<Box<RawValue>, ProjectionError> {
    let source: BTreeMap<String, Box<RawValue>> = serde_json::from_str(entry.get())?;
    let pricing_source: BTreeMap<String, Box<RawValue>> = source
        .iter()
        .filter(|(_, value)| value.get() != "null")
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    let pricing_raw = to_raw_value(&pricing_source)?;
    let parsed_fields: ModelFields = ModelFields::parse(&pricing_raw)
        .map_err(|error| ProjectionError::Fields(error.to_string()))?
        .0;
    drop(parsed_fields);
    PriceFields::parse(&pricing_raw).map_err(|error| ProjectionError::Prices(error.to_string()))?;
    let fixed: BTreeSet<&str> = FIXED_FIELDS.split_whitespace().collect();
    let mut result = fixed_fields(key, provider, &source)?;
    for (name, value) in source {
        if !fixed.contains(name.as_str()) && is_threshold_cost_key(&name) {
            result.insert(name, value);
        }
    }
    Ok(to_raw_value(&result)?)
}

fn fixed_fields(
    key: &str,
    provider: &str,
    source: &BTreeMap<String, Box<RawValue>>,
) -> Result<BTreeMap<String, Box<RawValue>>, ProjectionError> {
    let mut result = BTreeMap::new();
    for name in FIXED_FIELDS.split_whitespace() {
        let value = match name {
            "key" => to_raw_value(key)?,
            "input_cost_per_token" | "output_cost_per_token" => source
                .get(name)
                .filter(|value| value.get() != "null")
                .cloned()
                .unwrap_or(to_raw_value(&0_u8)?),
            "litellm_provider" => source
                .get(name)
                .cloned()
                .unwrap_or(to_raw_value(&provider)?),
            _ => source.get(name).cloned().unwrap_or(to_raw_value(&())?),
        };
        result.insert(name.to_owned(), value);
    }
    Ok(result)
}

fn is_threshold_cost_key(name: &str) -> bool {
    let Some(prefix) = name.strip_suffix("_tokens") else {
        return false;
    };
    let Some((_, number)) = prefix.rsplit_once("_above_") else {
        return false;
    };
    let digits = number.strip_suffix('k').unwrap_or(number);
    !digits.is_empty() && digits.chars().all(|character| character.is_ascii_digit())
}

mod tests;

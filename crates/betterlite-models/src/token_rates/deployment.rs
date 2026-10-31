use rust_decimal::Decimal;
use serde::Serialize;

use super::rates::cost_per_unit;
use super::{ModelInfo, TokenRateError};
use crate::tiered_pricing::Scalar;

// Short idents reuse the names crate::price_fields::values already gave these wire keys.
macro_rules! deployment_pricing {
    ($($field:ident: $wire:literal),+ $(,)?) => {
        // The fields of MirroredPricingParams/CustomPricingLiteLLMParams in litellm/types/utils.py, in order.
        #[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
        pub struct DeploymentPricing {
            $(
                #[serde(rename = $wire, skip_serializing_if = "Option::is_none")]
                pub $field: Option<Decimal>,
            )+
        }

        fn fill_deployment_pricing(
            deployment: &ModelInfo,
            pricing: &mut DeploymentPricing,
        ) -> Result<(), TokenRateError> {
            $(
                if !matches!(deployment.get($wire), None | Some(Scalar::Null)) {
                    pricing.$field = cost_per_unit(deployment, $wire, None)?;
                }
            )+
            Ok(())
        }
    };
}

deployment_pricing!(
    input_token: "input_cost_per_token",
    output_token: "output_cost_per_token",
    input_character: "input_cost_per_character",
    output_character: "output_cost_per_character",
    cache_read_input_token: "cache_read_input_token_cost",
    cache_write_input_token: "cache_creation_input_token_cost",
    tiered_pricing: "tiered_pricing",
    per_second: "cost_per_second",
    input_second: "input_cost_per_second",
    output_second: "output_cost_per_second",
    output_second_1080p: "output_cost_per_second_1080p",
    output_second_480p: "output_cost_per_second_480p",
    output_second_720p: "output_cost_per_second_720p",
    output_second_768p: "output_cost_per_second_768p",
    output_second_2k: "output_cost_per_second_2k",
    output_second_4k: "output_cost_per_second_4k",
    output_image_512: "output_cost_per_image_512",
    output_image_1024: "output_cost_per_image_1024",
    output_image_1536: "output_cost_per_image_1536",
    input_pixel: "input_cost_per_pixel",
    output_pixel: "output_cost_per_pixel",
    input_token_flex: "input_cost_per_token_flex",
    input_token_priority: "input_cost_per_token_priority",
    input_token_balanced: "input_cost_per_token_balanced",
    input_token_ultrafast: "input_cost_per_token_ultrafast",
    cache_write_over_1hr: "cache_creation_input_token_cost_above_1hr",
    cache_write_over_200k: "cache_creation_input_token_cost_above_200k_tokens",
    cache_write_over_272k: "cache_creation_input_token_cost_above_272k_tokens",
    cache_write_over_272k_priority: "cache_creation_input_token_cost_above_272k_tokens_priority",
    cache_write_over_272k_flex: "cache_creation_input_token_cost_above_272k_tokens_flex",
    cache_write_over_272k_ultra: "cache_creation_input_token_cost_above_272k_tokens_ultrafast",
    cache_write_input_token_flex: "cache_creation_input_token_cost_flex",
    cache_write_priority: "cache_creation_input_token_cost_priority",
    cache_write_ultrafast: "cache_creation_input_token_cost_ultrafast",
    cache_write_input_audio_token: "cache_creation_input_audio_token_cost",
    cache_read_input_token_flex: "cache_read_input_token_cost_flex",
    cache_read_priority: "cache_read_input_token_cost_priority",
    cache_read_balanced: "cache_read_input_token_cost_balanced",
    cache_read_ultrafast: "cache_read_input_token_cost_ultrafast",
    cache_read_over_200k: "cache_read_input_token_cost_above_200k_tokens",
    cache_read_over_200k_priority: "cache_read_input_token_cost_above_200k_tokens_priority",
    cache_read_over_272k_priority: "cache_read_input_token_cost_above_272k_tokens_priority",
    cache_read_over_272k_flex: "cache_read_input_token_cost_above_272k_tokens_flex",
    cache_read_over_272k_ultra: "cache_read_input_token_cost_above_272k_tokens_ultrafast",
    cache_read_input_token_batch: "cache_read_input_token_cost_batches",
    cache_read_over_200k_batch: "cache_read_input_token_cost_above_200k_tokens_batches",
    cache_read_over_272k_batch: "cache_read_input_token_cost_above_272k_tokens_batches",
    cache_write_input_token_batch: "cache_creation_input_token_cost_batches",
    cache_write_over_200k_batch: "cache_creation_input_token_cost_above_200k_tokens_batches",
    cache_write_over_272k_batch: "cache_creation_input_token_cost_above_272k_tokens_batches",
    cache_read_input_audio_token: "cache_read_input_audio_token_cost",
    cache_read_input_image_token: "cache_read_input_image_token_cost",
    input_character_over_128k: "input_cost_per_character_above_128k_tokens",
    input_audio_token: "input_cost_per_audio_token",
    input_token_cache_hit: "input_cost_per_token_cache_hit",
    input_token_over_128k: "input_cost_per_token_above_128k_tokens",
    input_token_over_200k: "input_cost_per_token_above_200k_tokens",
    input_token_over_200k_priority: "input_cost_per_token_above_200k_tokens_priority",
    input_token_over_272k_priority: "input_cost_per_token_above_272k_tokens_priority",
    input_token_over_272k_flex: "input_cost_per_token_above_272k_tokens_flex",
    input_over_272k_ultra: "input_cost_per_token_above_272k_tokens_ultrafast",
    input_token_over_200k_batch: "input_cost_per_token_above_200k_tokens_batches",
    input_token_over_272k_batch: "input_cost_per_token_above_272k_tokens_batches",
    input_query: "input_cost_per_query",
    input_image: "input_cost_per_image",
    input_image_over_128k: "input_cost_per_image_above_128k_tokens",
    input_audio_second: "input_cost_per_audio_per_second",
    input_audio_second_over_128k: "input_cost_per_audio_per_second_above_128k_tokens",
    input_video_second: "input_cost_per_video_per_second",
    input_video_second_over_128k: "input_cost_per_video_per_second_above_128k_tokens",
    input_video_second_over_15s: "input_cost_per_video_per_second_above_15s_interval",
    input_video_second_over_8s: "input_cost_per_video_per_second_above_8s_interval",
    input_audio_token_batch: "input_cost_per_audio_token_batches",
    input_image_token_batch: "input_cost_per_image_token_batches",
    input_token_batch: "input_cost_per_token_batches",
    input_video_token_batch: "input_cost_per_video_token_batches",
    output_token_batch: "output_cost_per_token_batches",
    output_token_flex: "output_cost_per_token_flex",
    output_token_priority: "output_cost_per_token_priority",
    output_token_balanced: "output_cost_per_token_balanced",
    output_token_ultrafast: "output_cost_per_token_ultrafast",
    output_audio_token: "output_cost_per_audio_token",
    output_token_over_128k: "output_cost_per_token_above_128k_tokens",
    output_token_over_200k: "output_cost_per_token_above_200k_tokens",
    output_over_200k_priority: "output_cost_per_token_above_200k_tokens_priority",
    output_over_272k_priority: "output_cost_per_token_above_272k_tokens_priority",
    output_token_over_272k_flex: "output_cost_per_token_above_272k_tokens_flex",
    output_over_272k_ultra: "output_cost_per_token_above_272k_tokens_ultrafast",
    output_token_over_200k_batch: "output_cost_per_token_above_200k_tokens_batches",
    output_token_over_272k_batch: "output_cost_per_token_above_272k_tokens_batches",
    output_character_over_128k: "output_cost_per_character_above_128k_tokens",
    output_image: "output_cost_per_image",
    output_image_token: "output_cost_per_image_token",
    output_video_token: "output_cost_per_video_token",
    output_reasoning_token: "output_cost_per_reasoning_token",
    output_reasoning_flex: "output_cost_per_reasoning_token_flex",
    output_reasoning_priority: "output_cost_per_reasoning_token_priority",
    output_video_second: "output_cost_per_video_per_second",
    output_audio_second: "output_cost_per_audio_per_second",
    search_context_query: "search_context_cost_per_query",
    google_maps_query: "google_maps_grounding_cost_per_query",
    citation_token: "citation_cost_per_token",
    cache_read_over_272k: "cache_read_input_token_cost_above_272k_tokens",
    cache_read_over_512k: "cache_read_input_token_cost_above_512k_tokens",
    input_image_token: "input_cost_per_image_token",
    input_video_token: "input_cost_per_video_token",
    input_token_over_272k: "input_cost_per_token_above_272k_tokens",
    input_token_over_512k: "input_cost_per_token_above_512k_tokens",
    output_token_over_272k: "output_cost_per_token_above_272k_tokens",
    output_token_over_512k: "output_cost_per_token_above_512k_tokens",
    output_vector_size: "output_vector_size",
    ocr_page: "ocr_cost_per_page",
    ocr_page_batch: "ocr_cost_per_page_batches",
    ocr_credit: "ocr_cost_per_credit",
    annotation_page: "annotation_cost_per_page",
    annotation_page_batch: "annotation_cost_per_page_batches",
    regional_processing_eu: "regional_processing_uplift_multiplier_eu",
    regional_processing_us: "regional_processing_uplift_multiplier_us",
    regional_endpoint_uplift: "regional_endpoint_uplift_multiplier",
);

// A nested table (tiered_pricing) or an unparsable string drops out silently, like an absent field.
pub fn deployment_pricing(
    model: Option<&ModelInfo>,
) -> Result<Option<DeploymentPricing>, TokenRateError> {
    let Some(deployment) = model else {
        return Ok(None);
    };
    let mut pricing = DeploymentPricing::default();
    fill_deployment_pricing(deployment, &mut pricing)?;
    Ok((pricing != DeploymentPricing::default()).then_some(pricing))
}

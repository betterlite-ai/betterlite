use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PricingValues {
    #[serde(
        rename = "input_cost_per_token",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub input_token: Option<Decimal>,
    #[serde(
        rename = "output_cost_per_token",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub output_token: Option<Decimal>,
    #[serde(
        rename = "input_cost_per_audio_token",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub input_audio_token: Option<Decimal>,
    #[serde(
        rename = "output_cost_per_audio_token",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub output_audio_token: Option<Decimal>,
    #[serde(
        rename = "input_cost_per_image",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub input_image: Option<Decimal>,
    #[serde(
        rename = "output_cost_per_image",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub output_image: Option<Decimal>,
    #[serde(
        rename = "input_cost_per_video_token",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub input_video_token: Option<Decimal>,
    #[serde(
        rename = "output_cost_per_video_token",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub output_video_token: Option<Decimal>,
    #[serde(
        rename = "input_cost_per_character",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub input_character: Option<Decimal>,
    #[serde(
        rename = "output_cost_per_character",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub output_character: Option<Decimal>,
    #[serde(
        rename = "input_cost_per_pixel",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub input_pixel: Option<Decimal>,
    #[serde(
        rename = "output_cost_per_pixel",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub output_pixel: Option<Decimal>,
    #[serde(
        rename = "input_cost_per_query",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub input_query: Option<Decimal>,
    #[serde(
        rename = "input_cost_per_request",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub input_request: Option<Decimal>,
    #[serde(
        rename = "input_cost_per_second",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub input_second: Option<Decimal>,
    #[serde(
        rename = "output_cost_per_second",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub output_second: Option<Decimal>,
    #[serde(
        rename = "cost_per_second",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub per_second: Option<Decimal>,
    #[serde(
        rename = "citation_cost_per_token",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub citation_token: Option<Decimal>,
    #[serde(
        rename = "code_interpreter_cost_per_session",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub code_interpreter_session: Option<Decimal>,
    #[serde(
        rename = "google_maps_grounding_cost_per_query",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub google_maps_query: Option<Decimal>,
    #[serde(
        rename = "ocr_cost_per_credit",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub ocr_credit: Option<Decimal>,
    #[serde(
        rename = "ocr_cost_per_page",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub ocr_page: Option<Decimal>,
    #[serde(
        rename = "regional_endpoint_uplift_multiplier",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub regional_endpoint_uplift: Option<Decimal>,
    #[serde(
        rename = "regional_processing_uplift_multiplier_eu",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub regional_processing_eu: Option<Decimal>,
    #[serde(
        rename = "regional_processing_uplift_multiplier_us",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub regional_processing_us: Option<Decimal>,
    #[serde(
        rename = "cache_creation_input_token_cost_above_128k_tokens",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub cache_create_input_over_12_0: Option<Decimal>,
    #[serde(
        rename = "cache_creation_input_token_cost_above_1hr",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub cache_create_input_over_1h_1: Option<Decimal>,
    #[serde(
        rename = "cache_creation_input_token_cost_above_1hr_above_200k_tokens",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub cache_create_input_over_1h_2: Option<Decimal>,
    #[serde(
        rename = "cache_creation_input_token_cost_above_200k_tokens",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub cache_create_input_over_20_3: Option<Decimal>,
    #[serde(
        rename = "cache_creation_input_token_cost_above_200k_tokens_batches",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub cache_create_input_over_20_4: Option<Decimal>,
    #[serde(
        rename = "cache_creation_input_token_cost_above_256k_tokens",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub cache_create_input_over_25_5: Option<Decimal>,
    #[serde(
        rename = "cache_creation_input_token_cost_above_272k_tokens",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub cache_create_input_over_27_6: Option<Decimal>,
    #[serde(
        rename = "cache_creation_input_token_cost_above_272k_tokens_batches",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub cache_create_input_over_27_7: Option<Decimal>,
    #[serde(
        rename = "cache_creation_input_token_cost_above_272k_tokens_flex",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub cache_create_input_over_27_8: Option<Decimal>,
    #[serde(
        rename = "cache_creation_input_token_cost_above_272k_tokens_priority",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub cache_create_input_over_27_9: Option<Decimal>,
    #[serde(
        rename = "cache_creation_input_token_cost_above_272k_tokens_ultrafast",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub cache_create_input_over_27_10: Option<Decimal>,
    #[serde(
        rename = "cache_creation_input_token_cost_above_32k_tokens",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub cache_create_input_over_32_11: Option<Decimal>,
    #[serde(
        rename = "cache_creation_input_token_cost_batches",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub cache_create_input_batch_12: Option<Decimal>,
    #[serde(
        rename = "cache_creation_input_token_cost_flex",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub cache_create_input_flex_13: Option<Decimal>,
    #[serde(
        rename = "cache_creation_input_token_cost_priority",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub cache_create_input_prio_14: Option<Decimal>,
    #[serde(
        rename = "cache_creation_input_token_cost_ultrafast",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub cache_create_input_ultra_15: Option<Decimal>,
    #[serde(
        rename = "cache_read_input_token_cost_above_128k_tokens",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub cache_read_input_over_128k_16: Option<Decimal>,
    #[serde(
        rename = "cache_read_input_token_cost_above_200k_tokens",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub cache_read_input_over_200k_17: Option<Decimal>,
    #[serde(
        rename = "cache_read_input_token_cost_above_200k_tokens_batches",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub cache_read_input_over_200k_18: Option<Decimal>,
    #[serde(
        rename = "cache_read_input_token_cost_above_200k_tokens_priority",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub cache_read_input_over_200k_19: Option<Decimal>,
    #[serde(
        rename = "cache_read_input_token_cost_above_256k_tokens",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub cache_read_input_over_256k_20: Option<Decimal>,
    #[serde(
        rename = "cache_read_input_token_cost_above_272k_tokens",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub cache_read_input_over_272k_21: Option<Decimal>,
    #[serde(
        rename = "cache_read_input_token_cost_above_272k_tokens_batches",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub cache_read_input_over_272k_22: Option<Decimal>,
    #[serde(
        rename = "cache_read_input_token_cost_above_272k_tokens_flex",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub cache_read_input_over_272k_23: Option<Decimal>,
    #[serde(
        rename = "cache_read_input_token_cost_above_272k_tokens_priority",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub cache_read_input_over_272k_24: Option<Decimal>,
    #[serde(
        rename = "cache_read_input_token_cost_above_272k_tokens_ultrafast",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub cache_read_input_over_272k_25: Option<Decimal>,
    #[serde(
        rename = "cache_read_input_token_cost_above_32k_tokens",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub cache_read_input_over_32k_26: Option<Decimal>,
    #[serde(
        rename = "cache_read_input_token_cost_above_512k_tokens",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub cache_read_input_over_512k_27: Option<Decimal>,
    #[serde(
        rename = "cache_read_input_token_cost_balanced",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub cache_read_input_balanced_28: Option<Decimal>,
    #[serde(
        rename = "cache_read_input_token_cost_batches",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub cache_read_input_batch_29: Option<Decimal>,
    #[serde(
        rename = "cache_read_input_token_cost_flex",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub cache_read_input_flex_30: Option<Decimal>,
    #[serde(
        rename = "cache_read_input_token_cost_priority",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub cache_read_input_prio_31: Option<Decimal>,
    #[serde(
        rename = "cache_read_input_token_cost_ultrafast",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub cache_read_input_ultra_32: Option<Decimal>,
    #[serde(
        rename = "input_cost_per_audio_per_second_above_128k_tokens",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub input_audio_per_second_ove_33: Option<Decimal>,
    #[serde(
        rename = "input_cost_per_audio_token_batches",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub input_audio_batch_34: Option<Decimal>,
    #[serde(
        rename = "input_cost_per_audio_token_priority",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub input_audio_prio_35: Option<Decimal>,
    #[serde(
        rename = "input_cost_per_character_above_128k_tokens",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub input_character_over_128k_36: Option<Decimal>,
    #[serde(
        rename = "input_cost_per_image_above_128k_tokens",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub input_image_over_128k_37: Option<Decimal>,
    #[serde(
        rename = "input_cost_per_image_token_batches",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub input_image_batch_38: Option<Decimal>,
    #[serde(
        rename = "input_cost_per_token_above_128k_tokens",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub input_over_128k_39: Option<Decimal>,
    #[serde(
        rename = "input_cost_per_token_above_200k_tokens",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub input_over_200k_40: Option<Decimal>,
    #[serde(
        rename = "input_cost_per_token_above_200k_tokens_batches",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub input_over_200k_batch_41: Option<Decimal>,
    #[serde(
        rename = "input_cost_per_token_above_200k_tokens_priority",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub input_over_200k_prio_42: Option<Decimal>,
    #[serde(
        rename = "input_cost_per_token_above_256k_tokens",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub input_over_256k_43: Option<Decimal>,
    #[serde(
        rename = "input_cost_per_token_above_272k_tokens",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub input_over_272k_44: Option<Decimal>,
    #[serde(
        rename = "input_cost_per_token_above_272k_tokens_batches",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub input_over_272k_batch_45: Option<Decimal>,
    #[serde(
        rename = "input_cost_per_token_above_272k_tokens_flex",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub input_over_272k_flex_46: Option<Decimal>,
    #[serde(
        rename = "input_cost_per_token_above_272k_tokens_priority",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub input_over_272k_prio_47: Option<Decimal>,
    #[serde(
        rename = "input_cost_per_token_above_272k_tokens_ultrafast",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub input_over_272k_ultra_48: Option<Decimal>,
    #[serde(
        rename = "input_cost_per_token_above_32k_tokens",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub input_over_32k_49: Option<Decimal>,
    #[serde(
        rename = "input_cost_per_token_above_512k_tokens",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub input_over_512k_50: Option<Decimal>,
    #[serde(
        rename = "input_cost_per_token_balanced",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub input_balanced_51: Option<Decimal>,
    #[serde(
        rename = "input_cost_per_token_batches",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub input_batch_52: Option<Decimal>,
    #[serde(
        rename = "input_cost_per_token_cache_hit",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub input_cache_hit_53: Option<Decimal>,
    #[serde(
        rename = "input_cost_per_token_flex",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub input_flex_54: Option<Decimal>,
    #[serde(
        rename = "input_cost_per_token_priority",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub input_prio_55: Option<Decimal>,
    #[serde(
        rename = "input_cost_per_token_ultrafast",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub input_ultra_56: Option<Decimal>,
    #[serde(
        rename = "input_cost_per_video_per_second",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub input_video_per_second_57: Option<Decimal>,
    #[serde(
        rename = "input_cost_per_video_per_second_above_128k_tokens",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub input_video_per_second_ove_58: Option<Decimal>,
    #[serde(
        rename = "input_cost_per_video_per_second_above_15s_interval",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub input_video_per_second_ove_59: Option<Decimal>,
    #[serde(
        rename = "input_cost_per_video_per_second_above_8s_interval",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub input_video_per_second_ove_60: Option<Decimal>,
    #[serde(
        rename = "input_cost_per_video_token_batches",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub input_video_batch_61: Option<Decimal>,
    #[serde(
        rename = "output_cost_per_character_above_128k_tokens",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub output_character_over_128k_62: Option<Decimal>,
    #[serde(
        rename = "output_cost_per_second_1080p",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub output_second_1080p_63: Option<Decimal>,
    #[serde(
        rename = "output_cost_per_second_2k",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub output_second_2k_64: Option<Decimal>,
    #[serde(
        rename = "output_cost_per_second_480p",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub output_second_480p_65: Option<Decimal>,
    #[serde(
        rename = "output_cost_per_second_4k",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub output_second_4k_66: Option<Decimal>,
    #[serde(
        rename = "output_cost_per_second_720p",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub output_second_720p_67: Option<Decimal>,
    #[serde(
        rename = "output_cost_per_second_768p",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub output_second_768p_68: Option<Decimal>,
    #[serde(
        rename = "output_cost_per_token_above_128k_tokens",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub output_over_128k_69: Option<Decimal>,
    #[serde(
        rename = "output_cost_per_token_above_200k_tokens",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub output_over_200k_70: Option<Decimal>,
    #[serde(
        rename = "output_cost_per_token_above_200k_tokens_batches",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub output_over_200k_batch_71: Option<Decimal>,
    #[serde(
        rename = "output_cost_per_token_above_200k_tokens_priority",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub output_over_200k_prio_72: Option<Decimal>,
    #[serde(
        rename = "output_cost_per_token_above_256k_tokens",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub output_over_256k_73: Option<Decimal>,
    #[serde(
        rename = "output_cost_per_token_above_272k_tokens",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub output_over_272k_74: Option<Decimal>,
    #[serde(
        rename = "output_cost_per_token_above_272k_tokens_batches",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub output_over_272k_batch_75: Option<Decimal>,
    #[serde(
        rename = "output_cost_per_token_above_272k_tokens_flex",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub output_over_272k_flex_76: Option<Decimal>,
    #[serde(
        rename = "output_cost_per_token_above_272k_tokens_priority",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub output_over_272k_prio_77: Option<Decimal>,
    #[serde(
        rename = "output_cost_per_token_above_272k_tokens_ultrafast",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub output_over_272k_ultra_78: Option<Decimal>,
    #[serde(
        rename = "output_cost_per_token_above_32k_tokens",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub output_over_32k_79: Option<Decimal>,
    #[serde(
        rename = "output_cost_per_token_above_512k_tokens",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub output_over_512k_80: Option<Decimal>,
    #[serde(
        rename = "output_cost_per_token_balanced",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub output_balanced_81: Option<Decimal>,
    #[serde(
        rename = "output_cost_per_token_batches",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub output_batch_82: Option<Decimal>,
    #[serde(
        rename = "output_cost_per_token_flex",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub output_flex_83: Option<Decimal>,
    #[serde(
        rename = "output_cost_per_token_priority",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub output_prio_84: Option<Decimal>,
    #[serde(
        rename = "output_cost_per_token_ultrafast",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::price_fields::decimal_option"
    )]
    pub output_ultra_85: Option<Decimal>,
}

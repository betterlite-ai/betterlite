use crate::price_fields::price_struct;
use serde::{Deserialize, Serialize};

price_struct! {
    #[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
    #[serde(default)]
    pub struct CachePricing {
        cache_write_input_audio_token: "cache_creation_input_audio_token_cost",
        cache_write_input_token: "cache_creation_input_token_cost",
        cache_write_over_128k: "cache_creation_input_token_cost_above_128k_tokens",
        cache_write_over_1hr: "cache_creation_input_token_cost_above_1hr",
        cache_write_over_1hr_over_200k: "cache_creation_input_token_cost_above_1hr_above_200k_tokens",
        cache_write_over_200k: "cache_creation_input_token_cost_above_200k_tokens",
        cache_write_over_200k_batch: "cache_creation_input_token_cost_above_200k_tokens_batches",
        cache_write_over_256k: "cache_creation_input_token_cost_above_256k_tokens",
        cache_write_over_272k: "cache_creation_input_token_cost_above_272k_tokens",
        cache_write_over_272k_batch: "cache_creation_input_token_cost_above_272k_tokens_batches",
        cache_write_over_272k_flex: "cache_creation_input_token_cost_above_272k_tokens_flex",
        cache_write_over_272k_priority: "cache_creation_input_token_cost_above_272k_tokens_priority",
        cache_write_over_272k_ultra: "cache_creation_input_token_cost_above_272k_tokens_ultrafast",
        cache_write_over_32k: "cache_creation_input_token_cost_above_32k_tokens",
        cache_write_input_token_batch: "cache_creation_input_token_cost_batches",
        cache_write_input_token_flex: "cache_creation_input_token_cost_flex",
        cache_write_priority: "cache_creation_input_token_cost_priority",
        cache_write_ultrafast: "cache_creation_input_token_cost_ultrafast",
        cache_read_input_audio_token: "cache_read_input_audio_token_cost",
        cache_read_input_image_token: "cache_read_input_image_token_cost",
        cache_read_input_token: "cache_read_input_token_cost",
        cache_read_over_128k: "cache_read_input_token_cost_above_128k_tokens",
        cache_read_over_200k: "cache_read_input_token_cost_above_200k_tokens",
        cache_read_over_200k_batch: "cache_read_input_token_cost_above_200k_tokens_batches",
        cache_read_over_200k_priority: "cache_read_input_token_cost_above_200k_tokens_priority",
        cache_read_over_256k: "cache_read_input_token_cost_above_256k_tokens",
        cache_read_over_272k: "cache_read_input_token_cost_above_272k_tokens",
        cache_read_over_272k_batch: "cache_read_input_token_cost_above_272k_tokens_batches",
        cache_read_over_272k_flex: "cache_read_input_token_cost_above_272k_tokens_flex",
        cache_read_over_272k_priority: "cache_read_input_token_cost_above_272k_tokens_priority",
        cache_read_over_272k_ultra: "cache_read_input_token_cost_above_272k_tokens_ultrafast",
        cache_read_over_32k: "cache_read_input_token_cost_above_32k_tokens",
        cache_read_over_512k: "cache_read_input_token_cost_above_512k_tokens",
        cache_read_balanced: "cache_read_input_token_cost_balanced",
        cache_read_input_token_batch: "cache_read_input_token_cost_batches",
        cache_read_input_token_flex: "cache_read_input_token_cost_flex",
        cache_read_priority: "cache_read_input_token_cost_priority",
        cache_read_ultrafast: "cache_read_input_token_cost_ultrafast",
    }
}

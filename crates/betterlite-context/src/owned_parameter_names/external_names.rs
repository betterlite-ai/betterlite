macro_rules! names {
    ($($registry:ident { $($name:literal),* }),*) => {
        $(pub(super) const $registry: &[&str] = &[$($name),*];)*
    };
}

names! {
    CALLBACK {
        "langfuse_public_key", "langfuse_secret", "langfuse_secret_key", "langfuse_host",
        "langfuse_environment", "langfuse_span_scope", "langfuse_prompt_version", "gcs_bucket_name",
        "gcs_path_service_account", "langsmith_api_key", "langsmith_project", "langsmith_base_url",
        "langsmith_sampling_rate", "langsmith_tenant_id", "humanloop_api_key", "arize_api_key",
        "arize_space_key", "arize_space_id", "arize_success_sampling_rate", "arize_error_sampling_rate",
        "posthog_api_key", "posthog_api_url", "wandb_api_key", "weave_project_id",
        "dd_api_key", "dd_site", "dd_agent_host", "dd_agent_port",
        "newrelic_api_key", "newrelic_region", "signoz_ingestion_endpoint", "signoz_ingestion_key",
        "turn_off_message_logging", "litellm_disabled_callbacks"
    },
    PRICING {
        "input_cost_per_token", "output_cost_per_token", "input_cost_per_character", "output_cost_per_character",
        "cache_read_input_token_cost", "cache_creation_input_token_cost", "tiered_pricing", "cost_per_second",
        "input_cost_per_second", "output_cost_per_second", "output_cost_per_second_1080p", "output_cost_per_second_480p",
        "output_cost_per_second_720p", "output_cost_per_second_768p", "output_cost_per_second_2k", "output_cost_per_second_4k",
        "output_cost_per_image_512", "output_cost_per_image_1024", "output_cost_per_image_1536", "input_cost_per_pixel",
        "output_cost_per_pixel", "input_cost_per_token_flex", "input_cost_per_token_priority", "input_cost_per_token_balanced",
        "input_cost_per_token_ultrafast", "cache_creation_input_token_cost_above_1hr", "cache_creation_input_token_cost_above_200k_tokens", "cache_creation_input_token_cost_above_272k_tokens",
        "cache_creation_input_token_cost_above_272k_tokens_priority", "cache_creation_input_token_cost_above_272k_tokens_flex", "cache_creation_input_token_cost_above_272k_tokens_ultrafast", "cache_creation_input_token_cost_flex",
        "cache_creation_input_token_cost_priority", "cache_creation_input_token_cost_ultrafast", "cache_creation_input_audio_token_cost", "cache_read_input_token_cost_flex",
        "cache_read_input_token_cost_priority", "cache_read_input_token_cost_balanced", "cache_read_input_token_cost_ultrafast", "cache_read_input_token_cost_above_200k_tokens",
        "cache_read_input_token_cost_above_200k_tokens_priority", "cache_read_input_token_cost_above_272k_tokens_priority", "cache_read_input_token_cost_above_272k_tokens_flex", "cache_read_input_token_cost_above_272k_tokens_ultrafast",
        "cache_read_input_token_cost_batches", "cache_read_input_token_cost_above_200k_tokens_batches", "cache_read_input_token_cost_above_272k_tokens_batches", "cache_creation_input_token_cost_batches",
        "cache_creation_input_token_cost_above_200k_tokens_batches", "cache_creation_input_token_cost_above_272k_tokens_batches", "cache_read_input_audio_token_cost", "cache_read_input_image_token_cost",
        "input_cost_per_character_above_128k_tokens", "input_cost_per_audio_token", "input_cost_per_token_cache_hit", "input_cost_per_token_above_128k_tokens",
        "input_cost_per_token_above_200k_tokens", "input_cost_per_token_above_200k_tokens_priority", "input_cost_per_token_above_272k_tokens_priority", "input_cost_per_token_above_272k_tokens_flex",
        "input_cost_per_token_above_272k_tokens_ultrafast", "input_cost_per_token_above_200k_tokens_batches", "input_cost_per_token_above_272k_tokens_batches", "input_cost_per_query",
        "input_cost_per_image", "input_cost_per_image_above_128k_tokens", "input_cost_per_audio_per_second", "input_cost_per_audio_per_second_above_128k_tokens",
        "input_cost_per_video_per_second", "input_cost_per_video_per_second_above_128k_tokens", "input_cost_per_video_per_second_above_15s_interval", "input_cost_per_video_per_second_above_8s_interval",
        "input_cost_per_audio_token_batches", "input_cost_per_image_token_batches", "input_cost_per_token_batches", "input_cost_per_video_token_batches",
        "output_cost_per_token_batches", "output_cost_per_token_flex", "output_cost_per_token_priority", "output_cost_per_token_balanced",
        "output_cost_per_token_ultrafast", "output_cost_per_audio_token", "output_cost_per_token_above_128k_tokens", "output_cost_per_token_above_200k_tokens",
        "output_cost_per_token_above_200k_tokens_priority", "output_cost_per_token_above_272k_tokens_priority", "output_cost_per_token_above_272k_tokens_flex", "output_cost_per_token_above_272k_tokens_ultrafast",
        "output_cost_per_token_above_200k_tokens_batches", "output_cost_per_token_above_272k_tokens_batches", "output_cost_per_character_above_128k_tokens", "output_cost_per_image",
        "output_cost_per_image_token", "output_cost_per_video_token", "output_cost_per_reasoning_token", "output_cost_per_reasoning_token_flex",
        "output_cost_per_reasoning_token_priority", "output_cost_per_video_per_second", "output_cost_per_audio_per_second", "search_context_cost_per_query",
        "google_maps_grounding_cost_per_query", "citation_cost_per_token", "cache_read_input_token_cost_above_272k_tokens", "cache_read_input_token_cost_above_512k_tokens",
        "input_cost_per_image_token", "input_cost_per_video_token", "input_cost_per_token_above_272k_tokens", "input_cost_per_token_above_512k_tokens",
        "output_cost_per_token_above_272k_tokens", "output_cost_per_token_above_512k_tokens", "output_vector_size", "ocr_cost_per_page",
        "ocr_cost_per_page_batches", "ocr_cost_per_credit", "annotation_cost_per_page", "annotation_cost_per_page_batches",
        "regional_processing_uplift_multiplier_eu", "regional_processing_uplift_multiplier_us", "regional_endpoint_uplift_multiplier"
    }
}

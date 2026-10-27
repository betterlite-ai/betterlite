#![cfg(test)]

use rstest::rstest;

use super::{RequestBanCatalog, observability};

const OBSERVABILITY_START: usize = 28;
const PRICING_START: usize = 65;

#[rstest]
#[case::api_base("api_base", 0)]
#[case::base_url("base_url", 1)]
#[case::user_config("user_config", 2)]
#[case::aws_sts_endpoint("aws_sts_endpoint", 3)]
#[case::aws_web_identity_token("aws_web_identity_token", 4)]
#[case::aws_role_name("aws_role_name", 5)]
#[case::aws_profile_name("aws_profile_name", 6)]
#[case::aws_session_name("aws_session_name", 7)]
#[case::aws_external_id("aws_external_id", 8)]
#[case::aws_session_tags("aws_session_tags", 9)]
#[case::vertex_credentials("vertex_credentials", 10)]
#[case::azure_ad_token("azure_ad_token", 11)]
#[case::aws_bedrock_runtime_endpoint("aws_bedrock_runtime_endpoint", 12)]
#[case::aws_bedrock_project_id("aws_bedrock_project_id", 13)]
#[case::workspace_id("workspace_id", 14)]
#[case::aws_workspace_id("aws_workspace_id", 15)]
#[case::anthropic_workspace_id("anthropic_workspace_id", 16)]
#[case::anthropic_workspace_id_hyphenated("anthropic-workspace-id", 17)]
#[case::bedrock_tags("bedrock_tags", 18)]
#[case::agentcore_runtime_session_id("agentcore_runtime_session_id", 19)]
#[case::s3_endpoint_url("s3_endpoint_url", 20)]
#[case::sagemaker_base_url("sagemaker_base_url", 21)]
#[case::deployment_url("deployment_url", 22)]
#[case::nvcf_function_id("nvcf_function_id", 23)]
#[case::use_ssl("use_ssl", 24)]
#[case::rust("rust", 25)]
#[case::model_list("model_list", 26)]
#[case::vertex_ai_credentials("vertex_ai_credentials", 27)]
fn base_field_is_banned_in_place(#[case] field: &str, #[case] index: usize) {
    assert_eq!(RequestBanCatalog::pinned().banned.get(index), Some(&field));
}

#[rstest]
#[case::langfuse_public_key("langfuse_public_key")]
#[case::langfuse_secret("langfuse_secret")]
#[case::langfuse_secret_key("langfuse_secret_key")]
#[case::langfuse_host("langfuse_host")]
#[case::langfuse_environment("langfuse_environment")]
#[case::langsmith_api_key("langsmith_api_key")]
#[case::langsmith_project("langsmith_project")]
#[case::langsmith_base_url("langsmith_base_url")]
#[case::langsmith_tenant_id("langsmith_tenant_id")]
#[case::humanloop_api_key("humanloop_api_key")]
#[case::arize_api_key("arize_api_key")]
#[case::arize_space_key("arize_space_key")]
#[case::arize_space_id("arize_space_id")]
#[case::arize_success_sampling_rate("arize_success_sampling_rate")]
#[case::arize_error_sampling_rate("arize_error_sampling_rate")]
#[case::posthog_api_key("posthog_api_key")]
#[case::posthog_host("posthog_host")]
#[case::braintrust_api_key("braintrust_api_key")]
#[case::braintrust_project("braintrust_project")]
#[case::braintrust_host("braintrust_host")]
#[case::slack_webhook_url("slack_webhook_url")]
#[case::lunary_public_key("lunary_public_key")]
#[case::dd_api_key("dd_api_key")]
#[case::dd_site("dd_site")]
#[case::dd_agent_host("dd_agent_host")]
#[case::dd_agent_port("dd_agent_port")]
#[case::newrelic_api_key("newrelic_api_key")]
#[case::newrelic_region("newrelic_region")]
#[case::signoz_ingestion_endpoint("signoz_ingestion_endpoint")]
#[case::signoz_ingestion_key("signoz_ingestion_key")]
#[case::turn_off_message_logging("turn_off_message_logging")]
#[case::gcs_bucket_name("gcs_bucket_name")]
#[case::gcs_path_service_account("gcs_path_service_account")]
#[case::posthog_api_url("posthog_api_url")]
#[case::user_api_key_auth_metadata("user_api_key_auth_metadata")]
#[case::wandb_api_key("wandb_api_key")]
#[case::weave_project_id("weave_project_id")]
fn observability_field_is_banned(#[case] field: &str) {
    let banned = RequestBanCatalog::pinned().banned;
    assert!(banned[OBSERVABILITY_START..PRICING_START].contains(&field));
}

#[rstest]
#[case::cost_per_second("cost_per_second")]
#[case::input_cost_per_second("input_cost_per_second")]
#[case::output_cost_per_second("output_cost_per_second")]
#[case::output_cost_per_second_1080p("output_cost_per_second_1080p")]
#[case::output_cost_per_second_480p("output_cost_per_second_480p")]
#[case::output_cost_per_second_720p("output_cost_per_second_720p")]
#[case::output_cost_per_second_768p("output_cost_per_second_768p")]
#[case::output_cost_per_second_2k("output_cost_per_second_2k")]
#[case::output_cost_per_second_4k("output_cost_per_second_4k")]
#[case::output_cost_per_image_512("output_cost_per_image_512")]
#[case::output_cost_per_image_1024("output_cost_per_image_1024")]
#[case::output_cost_per_image_1536("output_cost_per_image_1536")]
#[case::input_cost_per_pixel("input_cost_per_pixel")]
#[case::output_cost_per_pixel("output_cost_per_pixel")]
#[case::input_cost_per_token_flex("input_cost_per_token_flex")]
#[case::input_cost_per_token_priority("input_cost_per_token_priority")]
#[case::input_cost_per_token_balanced("input_cost_per_token_balanced")]
#[case::input_cost_per_token_ultrafast("input_cost_per_token_ultrafast")]
#[case::cache_creation_input_token_cost_above_1hr("cache_creation_input_token_cost_above_1hr")]
#[case::cache_creation_input_token_cost_above_200k_tokens(
    "cache_creation_input_token_cost_above_200k_tokens"
)]
#[case::cache_creation_input_token_cost_above_272k_tokens(
    "cache_creation_input_token_cost_above_272k_tokens"
)]
#[case::cache_creation_input_token_cost_above_272k_tokens_priority(
    "cache_creation_input_token_cost_above_272k_tokens_priority"
)]
#[case::cache_creation_input_token_cost_above_272k_tokens_flex(
    "cache_creation_input_token_cost_above_272k_tokens_flex"
)]
#[case::cache_creation_input_token_cost_above_272k_tokens_ultrafast(
    "cache_creation_input_token_cost_above_272k_tokens_ultrafast"
)]
#[case::cache_creation_input_token_cost_flex("cache_creation_input_token_cost_flex")]
#[case::cache_creation_input_token_cost_priority("cache_creation_input_token_cost_priority")]
#[case::cache_creation_input_token_cost_ultrafast("cache_creation_input_token_cost_ultrafast")]
#[case::cache_creation_input_audio_token_cost("cache_creation_input_audio_token_cost")]
#[case::cache_read_input_token_cost_flex("cache_read_input_token_cost_flex")]
#[case::cache_read_input_token_cost_priority("cache_read_input_token_cost_priority")]
#[case::cache_read_input_token_cost_balanced("cache_read_input_token_cost_balanced")]
#[case::cache_read_input_token_cost_ultrafast("cache_read_input_token_cost_ultrafast")]
#[case::cache_read_input_token_cost_above_200k_tokens(
    "cache_read_input_token_cost_above_200k_tokens"
)]
#[case::cache_read_input_token_cost_above_200k_tokens_priority(
    "cache_read_input_token_cost_above_200k_tokens_priority"
)]
#[case::cache_read_input_token_cost_above_272k_tokens_priority(
    "cache_read_input_token_cost_above_272k_tokens_priority"
)]
#[case::cache_read_input_token_cost_above_272k_tokens_flex(
    "cache_read_input_token_cost_above_272k_tokens_flex"
)]
#[case::cache_read_input_token_cost_above_272k_tokens_ultrafast(
    "cache_read_input_token_cost_above_272k_tokens_ultrafast"
)]
#[case::cache_read_input_token_cost_batches("cache_read_input_token_cost_batches")]
#[case::cache_read_input_token_cost_above_200k_tokens_batches(
    "cache_read_input_token_cost_above_200k_tokens_batches"
)]
#[case::cache_read_input_token_cost_above_272k_tokens_batches(
    "cache_read_input_token_cost_above_272k_tokens_batches"
)]
#[case::cache_creation_input_token_cost_batches("cache_creation_input_token_cost_batches")]
#[case::cache_creation_input_token_cost_above_200k_tokens_batches(
    "cache_creation_input_token_cost_above_200k_tokens_batches"
)]
#[case::cache_creation_input_token_cost_above_272k_tokens_batches(
    "cache_creation_input_token_cost_above_272k_tokens_batches"
)]
#[case::cache_read_input_audio_token_cost("cache_read_input_audio_token_cost")]
#[case::cache_read_input_image_token_cost("cache_read_input_image_token_cost")]
#[case::input_cost_per_character_above_128k_tokens("input_cost_per_character_above_128k_tokens")]
#[case::input_cost_per_audio_token("input_cost_per_audio_token")]
#[case::input_cost_per_token_cache_hit("input_cost_per_token_cache_hit")]
#[case::input_cost_per_token_above_128k_tokens("input_cost_per_token_above_128k_tokens")]
#[case::input_cost_per_token_above_200k_tokens("input_cost_per_token_above_200k_tokens")]
#[case::input_cost_per_token_above_200k_tokens_priority(
    "input_cost_per_token_above_200k_tokens_priority"
)]
#[case::input_cost_per_token_above_272k_tokens_priority(
    "input_cost_per_token_above_272k_tokens_priority"
)]
#[case::input_cost_per_token_above_272k_tokens_flex("input_cost_per_token_above_272k_tokens_flex")]
#[case::input_cost_per_token_above_272k_tokens_ultrafast(
    "input_cost_per_token_above_272k_tokens_ultrafast"
)]
#[case::input_cost_per_token_above_200k_tokens_batches(
    "input_cost_per_token_above_200k_tokens_batches"
)]
#[case::input_cost_per_token_above_272k_tokens_batches(
    "input_cost_per_token_above_272k_tokens_batches"
)]
#[case::input_cost_per_query("input_cost_per_query")]
#[case::input_cost_per_image("input_cost_per_image")]
#[case::input_cost_per_image_above_128k_tokens("input_cost_per_image_above_128k_tokens")]
#[case::input_cost_per_audio_per_second("input_cost_per_audio_per_second")]
#[case::input_cost_per_audio_per_second_above_128k_tokens(
    "input_cost_per_audio_per_second_above_128k_tokens"
)]
#[case::input_cost_per_video_per_second("input_cost_per_video_per_second")]
#[case::input_cost_per_video_per_second_above_128k_tokens(
    "input_cost_per_video_per_second_above_128k_tokens"
)]
#[case::input_cost_per_video_per_second_above_15s_interval(
    "input_cost_per_video_per_second_above_15s_interval"
)]
#[case::input_cost_per_video_per_second_above_8s_interval(
    "input_cost_per_video_per_second_above_8s_interval"
)]
#[case::input_cost_per_audio_token_batches("input_cost_per_audio_token_batches")]
#[case::input_cost_per_image_token_batches("input_cost_per_image_token_batches")]
#[case::input_cost_per_token_batches("input_cost_per_token_batches")]
#[case::input_cost_per_video_token_batches("input_cost_per_video_token_batches")]
#[case::output_cost_per_token_batches("output_cost_per_token_batches")]
#[case::output_cost_per_token_flex("output_cost_per_token_flex")]
#[case::output_cost_per_token_priority("output_cost_per_token_priority")]
#[case::output_cost_per_token_balanced("output_cost_per_token_balanced")]
#[case::output_cost_per_token_ultrafast("output_cost_per_token_ultrafast")]
#[case::output_cost_per_audio_token("output_cost_per_audio_token")]
#[case::output_cost_per_token_above_128k_tokens("output_cost_per_token_above_128k_tokens")]
#[case::output_cost_per_token_above_200k_tokens("output_cost_per_token_above_200k_tokens")]
#[case::output_cost_per_token_above_200k_tokens_priority(
    "output_cost_per_token_above_200k_tokens_priority"
)]
#[case::output_cost_per_token_above_272k_tokens_priority(
    "output_cost_per_token_above_272k_tokens_priority"
)]
#[case::output_cost_per_token_above_272k_tokens_flex(
    "output_cost_per_token_above_272k_tokens_flex"
)]
#[case::output_cost_per_token_above_272k_tokens_ultrafast(
    "output_cost_per_token_above_272k_tokens_ultrafast"
)]
#[case::output_cost_per_token_above_200k_tokens_batches(
    "output_cost_per_token_above_200k_tokens_batches"
)]
#[case::output_cost_per_token_above_272k_tokens_batches(
    "output_cost_per_token_above_272k_tokens_batches"
)]
#[case::output_cost_per_character_above_128k_tokens("output_cost_per_character_above_128k_tokens")]
#[case::output_cost_per_image("output_cost_per_image")]
#[case::output_cost_per_image_token("output_cost_per_image_token")]
#[case::output_cost_per_video_token("output_cost_per_video_token")]
#[case::output_cost_per_reasoning_token("output_cost_per_reasoning_token")]
#[case::output_cost_per_reasoning_token_flex("output_cost_per_reasoning_token_flex")]
#[case::output_cost_per_reasoning_token_priority("output_cost_per_reasoning_token_priority")]
#[case::output_cost_per_video_per_second("output_cost_per_video_per_second")]
#[case::output_cost_per_audio_per_second("output_cost_per_audio_per_second")]
#[case::search_context_cost_per_query("search_context_cost_per_query")]
#[case::google_maps_grounding_cost_per_query("google_maps_grounding_cost_per_query")]
#[case::citation_cost_per_token("citation_cost_per_token")]
#[case::cache_read_input_token_cost_above_272k_tokens(
    "cache_read_input_token_cost_above_272k_tokens"
)]
#[case::cache_read_input_token_cost_above_512k_tokens(
    "cache_read_input_token_cost_above_512k_tokens"
)]
#[case::input_cost_per_image_token("input_cost_per_image_token")]
#[case::input_cost_per_video_token("input_cost_per_video_token")]
#[case::input_cost_per_token_above_272k_tokens("input_cost_per_token_above_272k_tokens")]
#[case::input_cost_per_token_above_512k_tokens("input_cost_per_token_above_512k_tokens")]
#[case::output_cost_per_token_above_272k_tokens("output_cost_per_token_above_272k_tokens")]
#[case::output_cost_per_token_above_512k_tokens("output_cost_per_token_above_512k_tokens")]
#[case::output_vector_size("output_vector_size")]
#[case::ocr_cost_per_page("ocr_cost_per_page")]
#[case::ocr_cost_per_page_batches("ocr_cost_per_page_batches")]
#[case::ocr_cost_per_credit("ocr_cost_per_credit")]
#[case::annotation_cost_per_page("annotation_cost_per_page")]
#[case::annotation_cost_per_page_batches("annotation_cost_per_page_batches")]
#[case::regional_processing_uplift_multiplier_eu("regional_processing_uplift_multiplier_eu")]
#[case::regional_processing_uplift_multiplier_us("regional_processing_uplift_multiplier_us")]
#[case::regional_endpoint_uplift_multiplier("regional_endpoint_uplift_multiplier")]
fn pricing_field_is_banned(#[case] field: &str) {
    let banned = RequestBanCatalog::pinned().banned;
    assert!(banned[PRICING_START..].contains(&field));
}

#[rstest]
#[case::input_cost_per_token("input_cost_per_token")]
#[case::output_cost_per_token("output_cost_per_token")]
#[case::input_cost_per_character("input_cost_per_character")]
#[case::output_cost_per_character("output_cost_per_character")]
#[case::cache_read_input_token_cost("cache_read_input_token_cost")]
#[case::cache_creation_input_token_cost("cache_creation_input_token_cost")]
#[case::tiered_pricing("tiered_pricing")]
fn inherited_pricing_included(#[case] field: &str) {
    let banned = RequestBanCatalog::pinned().banned;
    assert!(banned[PRICING_START..].contains(&field));
}

#[test]
fn ordered_base_then_sorted_sets() {
    let banned = RequestBanCatalog::pinned().banned;
    let observability = &banned[OBSERVABILITY_START..PRICING_START];
    let pricing = &banned[PRICING_START..];
    assert_eq!(banned.len(), 184);
    assert!(observability.is_sorted_by(|left, right| left < right));
    assert!(pricing.is_sorted_by(|left, right| left < right));
    assert_eq!(observability.first(), Some(&"arize_api_key"));
    assert_eq!(pricing.first(), Some(&"annotation_cost_per_page"));
    assert_eq!(pricing.last(), Some(&"tiered_pricing"));
}

#[test]
fn shared_blocked_fields_appear_once() {
    let banned = RequestBanCatalog::pinned().banned;
    assert_eq!(
        banned
            .iter()
            .filter(|field| **field == "dd_api_key")
            .count(),
        1
    );
    assert_eq!(observability::banned_fields().len(), 37);
}

#[rstest]
#[case::langfuse_prompt_version("langfuse_prompt_version")]
#[case::langsmith_sampling_rate("langsmith_sampling_rate")]
fn safe_callback_exclusions(#[case] field: &str) {
    assert!(!RequestBanCatalog::pinned().banned.contains(&field));
}

#[rstest]
#[case::phoenix_project_name("phoenix_project_name")]
#[case::phoenix_project_name_override("phoenix_project_name_override")]
fn phoenix_inert_fields_absent(#[case] field: &str) {
    assert!(!RequestBanCatalog::pinned().banned.contains(&field));
}

#[test]
fn nested_slot_order() {
    let catalog = RequestBanCatalog::pinned();
    assert_eq!(
        catalog.nested_config,
        ["litellm_embedding_config", "extra_body"]
    );
    assert_eq!(catalog.metadata, ["metadata", "litellm_metadata"]);
}

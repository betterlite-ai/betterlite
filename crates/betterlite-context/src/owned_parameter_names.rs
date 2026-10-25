pub const INTERNAL_KWARG_PREFIX: &str = "_litellm_";
const CONNECTION: &str = "api_key api_base api_version region_name headers provider_specific_header client shared_session ssl_verify request_timeout force_timeout stream_timeout max_retries tenant_id client_id client_secret azure_username azure_password azure_scope azure_ad_token_provider litellm_credential_name configurable_clientside_auth_params use_xai_oauth";
const BATCH: &str = "aws_batch_role_arn s3_bucket_name s3_region_name s3_endpoint_url s3_output_bucket_name s3_bucket_owner s3_access_key_id s3_secret_access_key s3_encryption_key_id bedrock_tags";
const OPTIONS: &str = "custom_llm_provider azure use_litellm_proxy use_chat_completions_api use_in_pass_through allowed_openai_params fallbacks context_window_fallback_dict num_retries retry_policy retry_strategy routing_strategy cooldown_time allowed_model_region enable_tag_filtering fastest_response provider_affinity_header search_tool_name model_list model_info rpm tpm itpm otpm default_api_key_rpm_limit default_api_key_tpm_limit max_parallel_requests weight order tag_regex max_file_size_mb silent_model auto_router_config_path auto_router_config auto_router_default_model auto_router_embedding_model auto_router_max_input_chars auto_router_routing_compression auto_router_model_compression complexity_router_config complexity_router_default_model adaptive_router_config adaptive_router_default_model quality_router_config quality_router_default_model caching cache ttl enable_prompt_caching caching_groups cost_per_query base_model max_budget budget_duration id metadata litellm_metadata tags litellm_trace_id litellm_session_id litellm_request_debug logger_fn verbose no-log log_client_error_tracebacks max_agentic_loops guardrails prompt_id prompt_variables prompt_version prompt_environment prompt_label litellm_system_prompt custom_prompt_dict roles final_prompt_value bos_token eos_token hf_model_name supports_system_message ensure_alternating_roles user_continue_message assistant_continue_message disable_add_transform_inline_image_block merge_reasoning_content_in_choices enable_json_schema_validation complete_response keepalive_seconds allow_client_keepalive_override stream_chunk_size mock_response mock_timeout";
const INTERNAL: &str = "litellm_call_id completion_call_id model_alias_map data_residency litellm_logging_obj preset_cache_key cache_key stream_response _context_compaction_state _agentic_loop_depth _agentic_loop_fingerprints _agentic_loop_api_surface _code_interpreter_interception_active _code_interpreter_interception_sandbox_key _code_interpreter_interception_session_scoped _code_interpreter_interception_converted_stream _websearch_interception_emit_native_blocks _websearch_interception_converted_stream _headroom_interception_converted_stream _router_weights fallback_depth max_fallbacks attempted_targets proxy_server_request secret_fields litellm_trusted_callback_vars _litellm_addressed_response_id _litellm_strip_stream_usage client_side_timeout model_file_id_mapping acompletion aembedding aimg_generation atext_completion text_completion allm_passthrough_route async_call";
const CALLBACK: &str = "langfuse_public_key langfuse_secret langfuse_secret_key langfuse_host langfuse_environment langfuse_span_scope langfuse_prompt_version gcs_bucket_name gcs_path_service_account langsmith_api_key langsmith_project langsmith_base_url langsmith_sampling_rate langsmith_tenant_id humanloop_api_key arize_api_key arize_space_key arize_space_id arize_success_sampling_rate arize_error_sampling_rate posthog_api_key posthog_api_url wandb_api_key weave_project_id dd_api_key dd_site dd_agent_host dd_agent_port newrelic_api_key newrelic_region signoz_ingestion_endpoint signoz_ingestion_key turn_off_message_logging litellm_disabled_callbacks";
const PRICING: &str = "input_cost_per_token output_cost_per_token input_cost_per_character output_cost_per_character cache_read_input_token_cost cache_creation_input_token_cost tiered_pricing cost_per_second input_cost_per_second output_cost_per_second output_cost_per_second_1080p output_cost_per_second_480p output_cost_per_second_720p output_cost_per_second_768p output_cost_per_second_2k output_cost_per_second_4k output_cost_per_image_512 output_cost_per_image_1024 output_cost_per_image_1536 input_cost_per_pixel output_cost_per_pixel input_cost_per_token_flex input_cost_per_token_priority input_cost_per_token_balanced input_cost_per_token_ultrafast cache_creation_input_token_cost_above_1hr cache_creation_input_token_cost_above_200k_tokens cache_creation_input_token_cost_above_272k_tokens cache_creation_input_token_cost_above_272k_tokens_priority cache_creation_input_token_cost_above_272k_tokens_flex cache_creation_input_token_cost_above_272k_tokens_ultrafast cache_creation_input_token_cost_flex cache_creation_input_token_cost_priority cache_creation_input_token_cost_ultrafast cache_creation_input_audio_token_cost cache_read_input_token_cost_flex cache_read_input_token_cost_priority cache_read_input_token_cost_balanced cache_read_input_token_cost_ultrafast cache_read_input_token_cost_above_200k_tokens cache_read_input_token_cost_above_200k_tokens_priority cache_read_input_token_cost_above_272k_tokens_priority cache_read_input_token_cost_above_272k_tokens_flex cache_read_input_token_cost_above_272k_tokens_ultrafast cache_read_input_token_cost_batches cache_read_input_token_cost_above_200k_tokens_batches cache_read_input_token_cost_above_272k_tokens_batches cache_creation_input_token_cost_batches cache_creation_input_token_cost_above_200k_tokens_batches cache_creation_input_token_cost_above_272k_tokens_batches cache_read_input_audio_token_cost cache_read_input_image_token_cost input_cost_per_character_above_128k_tokens input_cost_per_audio_token input_cost_per_token_cache_hit input_cost_per_token_above_128k_tokens input_cost_per_token_above_200k_tokens input_cost_per_token_above_200k_tokens_priority input_cost_per_token_above_272k_tokens input_cost_per_token_above_272k_tokens_priority input_cost_per_token_above_272k_tokens_flex input_cost_per_token_above_272k_tokens_ultrafast input_cost_per_token_above_200k_tokens_batches input_cost_per_token_above_272k_tokens_batches input_cost_per_query input_cost_per_image input_cost_per_image_above_128k_tokens input_cost_per_audio_per_second input_cost_per_audio_per_second_above_128k_tokens input_cost_per_video_per_second input_cost_per_video_per_second_above_128k_tokens input_cost_per_video_per_second_above_15s_interval input_cost_per_video_per_second_above_8s_interval input_cost_per_audio_token_batches input_cost_per_image_token_batches input_cost_per_token_batches input_cost_per_video_token_batches output_cost_per_token_batches output_cost_per_token_flex output_cost_per_token_priority output_cost_per_token_balanced output_cost_per_token_ultrafast output_cost_per_audio_token output_cost_per_token_above_128k_tokens output_cost_per_token_above_200k_tokens output_cost_per_token_above_200k_tokens_priority output_cost_per_token_above_272k_tokens output_cost_per_token_above_272k_tokens_priority output_cost_per_token_above_272k_tokens_flex output_cost_per_token_above_272k_tokens_ultrafast output_cost_per_token_above_200k_tokens_batches output_cost_per_token_above_272k_tokens_batches output_cost_per_character_above_128k_tokens output_cost_per_image output_cost_per_image_token output_cost_per_video_token output_cost_per_reasoning_token output_cost_per_reasoning_token_flex output_cost_per_reasoning_token_priority output_cost_per_video_per_second output_cost_per_audio_per_second search_context_cost_per_query google_maps_grounding_cost_per_query citation_cost_per_token cache_read_input_token_cost_above_272k_tokens cache_read_input_token_cost_above_512k_tokens input_cost_per_image_token input_cost_per_video_token input_cost_per_token_above_272k_tokens input_cost_per_token_above_512k_tokens output_cost_per_token_above_272k_tokens output_cost_per_token_above_512k_tokens output_vector_size ocr_cost_per_page ocr_cost_per_page_batches ocr_cost_per_credit annotation_cost_per_page annotation_cost_per_page_batches regional_processing_uplift_multiplier_eu regional_processing_uplift_multiplier_us regional_endpoint_uplift_multiplier";

fn words(text: &str) -> Vec<String> {
    let names = text.split_whitespace().map(str::to_owned).collect();
    names
}
fn pricing_names() -> Vec<String> {
    let mut names = words(PRICING);
    if let Some(index) = names
        .iter()
        .position(|name| name == "input_cost_per_token_above_272k_tokens")
    {
        names.remove(index);
    }
    if let Some(index) = names
        .iter()
        .position(|name| name == "output_cost_per_token_above_272k_tokens")
    {
        names.remove(index);
    }
    names
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct WireMetadata {
    pub wire_name: String,
}

#[must_use]
pub fn wire(name: &str) -> WireMetadata {
    let wire_name = name.to_owned();
    WireMetadata { wire_name }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Owner {
    ConnectionSettings,
    LiteLLMOptions,
    InternalState,
    Observability,
    AgenticLoopState,
    AgenticLoopOptions,
    #[serde(rename = "bedrock_batch")]
    BedrockBatchConnection,
    StandardCallbackDynamicParams,
    CustomPricingLiteLLMParams,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnedRoot {
    ConnectionSettings,
    LiteLLMOptions,
    InternalState,
    InvalidLeaf,
}

#[derive(Debug, thiserror::Error)]
pub enum OwnedParameterNamesError {
    #[error("{root}.{field} is not a dataclass leaf")]
    NotDataclassLeaf {
        root: &'static str,
        field: &'static str,
    },
}

const fn owner_fields(owner: Owner) -> &'static str {
    match owner {
        Owner::ConnectionSettings => CONNECTION,
        Owner::LiteLLMOptions => OPTIONS,
        Owner::InternalState => INTERNAL,
        Owner::Observability => {
            "id metadata litellm_metadata tags litellm_trace_id litellm_session_id litellm_request_debug logger_fn verbose no-log log_client_error_tracebacks"
        }
        Owner::AgenticLoopState => {
            "_agentic_loop_depth _agentic_loop_fingerprints _agentic_loop_api_surface _code_interpreter_interception_active _code_interpreter_interception_sandbox_key _code_interpreter_interception_session_scoped _code_interpreter_interception_converted_stream _websearch_interception_emit_native_blocks _websearch_interception_converted_stream _headroom_interception_converted_stream"
        }
        Owner::AgenticLoopOptions => "max_agentic_loops",
        Owner::BedrockBatchConnection => BATCH,
        Owner::StandardCallbackDynamicParams => CALLBACK,
        Owner::CustomPricingLiteLLMParams => PRICING,
    }
}

pub fn wire_names(owner: Owner) -> Result<Vec<String>, OwnedParameterNamesError> {
    let fields = owner_fields(owner);
    Ok(words(fields))
}

pub fn owned_wire_names(root: OwnedRoot) -> Result<Vec<String>, OwnedParameterNamesError> {
    let leaves = match root {
        OwnedRoot::ConnectionSettings => [
            Some(Owner::ConnectionSettings),
            Some(Owner::BedrockBatchConnection),
        ],
        OwnedRoot::LiteLLMOptions => [Some(Owner::LiteLLMOptions), None],
        OwnedRoot::InternalState => [Some(Owner::InternalState), None],
        OwnedRoot::InvalidLeaf => {
            return Err(OwnedParameterNamesError::NotDataclassLeaf {
                root: "InvalidLeaf",
                field: "value",
            });
        }
    };
    let mut names = Vec::new();
    for leaf in leaves.into_iter().flatten() {
        names.extend(wire_names(leaf)?);
    }
    Ok(names)
}

pub fn owned_names() -> Result<Vec<String>, OwnedParameterNamesError> {
    let mut names = owned_wire_names(OwnedRoot::ConnectionSettings)?;
    names.extend(owned_wire_names(OwnedRoot::LiteLLMOptions)?);
    names.extend(owned_wire_names(OwnedRoot::InternalState)?);
    names.extend(words("self use_client model_config rust"));
    names.extend(wire_names(Owner::StandardCallbackDynamicParams)?);
    names.extend(pricing_names());
    Ok(names)
}

pub fn is_litellm_owned_kwarg(name: &str) -> Result<bool, OwnedParameterNamesError> {
    if name.starts_with(INTERNAL_KWARG_PREFIX) {
        return Ok(true);
    }
    Ok(owned_names()?.iter().any(|owned| owned == name))
}

#[path = "owned_parameter_names/tests.rs"]
mod owned_parameter_names_cases;

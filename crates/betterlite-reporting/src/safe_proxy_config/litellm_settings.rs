pub(crate) const LITELLM_SETTINGS_KEYS: &str = "* AI21ChatConfig AI21Config APIConnectionError APIError APIResponseValidationError \
    AZURE_DEFAULT_API_VERSION AdapterItem AlephAlphaConfig AmazonTitanV2Config AnthropicModelInfo \
    Any AuthenticationError BEDROCK_CONVERSE_MODELS BEDROCK_EMBEDDING_PROVIDERS_LITERAL \
    BEDROCK_INVOKE_PROVIDERS_LITERAL BadGatewayError BadRequestError BudgetExceededError \
    BudgetManager CALLBACK_TYPES COHERE_DEFAULT_EMBEDDING_INPUT_TYPE Callable \
    ContentPolicyViolationError ContextWindowExceededError CustomLLM CustomLLMItem \
    DEFAULT_ALLOWED_FAILS DEFAULT_BATCH_SIZE DEFAULT_FLUSH_INTERVAL_SECONDS DEFAULT_MAX_RETRIES \
    DEFAULT_MAX_TOKENS DEFAULT_REPLICATE_POLLING_DELAY_SECONDS DEFAULT_REPLICATE_POLLING_RETRIES \
    DEFAULT_SOFT_BUDGET DatadogInitParams Dict Final GeminiModelInfo \
    HUMANLOOP_PROMPT_CACHE_TTL_SECONDS ImageFetchError InternalServerError InvalidRequestError \
    JSONSchemaValidationError KeyManagementSettings LITELLM_CHAT_PROVIDERS LITELLM_EXCEPTION_TYPES \
    List Literal LlmProviders Mapping MockException ModelNotMappedError NewRelicInitParams \
    NotFoundError OPENAI_CHAT_COMPLETION_PARAMS OPENAI_FINISH_REASONS OpenAIError Optional \
    PalmConfig PermissionDeniedError PointFiveInitParams PromptSpec REPEATED_STREAMING_CHUNK_LIMIT \
    ROUTER_MAX_FALLBACKS RateLimitError RateLimitErrorCategory RateLimitType Router Sequence \
    ServiceUnavailableError Set TYPE_CHECKING Timeout TopazModelInfo Tuple Type Union \
    UnprocessableEntityError UnsupportedParamsError VectorStoreIndexRegistry VectorStoreRegistry \
    VectorStoreSearchError VectorStoreSearchFailureMode VertexAITextEmbeddingConfig WANDB_MODELS \
    WATSONX_DEFAULT_API_VERSION XAIModelInfo ZerobusInitParams _AGENT_EXPORTS \
    _anthropic_prompt_caching_ttl_env _arealtime _aresponses_websocket \
    _async_client_cleanup_registered _async_failure_callback _async_input_callback \
    _async_success_callback _current_cost _custom_logger_compatible_callbacks_literal \
    _custom_providers _google_kms_resource_name _key_management_settings _key_management_system \
    _known_custom_logger_compatible_callbacks _openai_completion_params _openai_finish_reasons \
    _openai_like_providers _thread_context _turn_on_debug _turn_on_json acancel_eval \
    acancel_responses acancel_run aclient_session acompact_responses acreate_agent acreate_eval \
    acreate_realtime_client_secret acreate_realtime_transcription_session acreate_run acreate_skill \
    adapters add_function_to_prompt add_user_information_to_llm_headers adelete_agent adelete_eval \
    adelete_responses adelete_run adelete_skill agenerate_content agent_search_embedding_model \
    aget_agent aget_eval aget_responses aget_run aget_skill ai21_chat_models ai21_key ai21_models \
    aiml_models aiohttp_trust_env aleph_alpha_key aleph_alpha_models alist_agent_versions \
    alist_agents alist_evals alist_input_items alist_runs alist_skills all_embedding_models \
    allm_passthrough_route allow_dynamic_callback_disabling allowed_fails amazon_nova_api_key \
    amazon_nova_models annotations anthropic_beta_headers_url anthropic_key anthropic_models \
    anthropic_prompt_caching_ttl anthropic_sse_ping_interval_seconds anyscale_models api_base \
    api_key api_version arealtime_calls aresponses_api_with_mcp argilla_batch_size \
    argilla_transformation_object assemblyai_models audit_log_callbacks autorouter_presets_url \
    avector_store_file_content avector_store_file_create avector_store_file_delete \
    avector_store_file_list avector_store_file_retrieve avector_store_file_update aws_polly_models \
    aws_sqs_callback_params azure_ai_models azure_anthropic_models azure_embedding_models azure_key \
    azure_llms azure_models azure_text_models banned_keywords_list baseten_key baseten_models \
    bedrock_converse_models bedrock_embedding_models bedrock_mantle_models bedrock_models \
    bedrock_neutralize_orphaned_tool_blocks bedrock_request_metadata_fields \
    black_forest_labs_models block_requests_for_models_without_pricing blocked_user_list \
    blog_posts_url budget_duration budget_exceeded_status_code budget_exceeded_throttle_percentage \
    budget_rollover bytez_key cache caching caching_with_models callback_settings callbacks \
    cancel_eval cancel_responses cancel_run cerebras_models chatgpt_models check_provider_endpoint \
    clarifai_key clarifai_models client client_session close_litellm_async_clients \
    cloudflare_api_key cloudflare_models codestral_models cohere_chat_models \
    cohere_embedding_models cohere_key cohere_models cold_storage_custom_logger cometapi_key \
    cometapi_models common_cloud_provider_auth_params compact_responses compress config_path \
    content_policy_fallbacks context_window_fallbacks cost_discount_config cost_margin_config \
    create_agent create_eval create_run create_skill credential_list \
    custom_prometheus_metadata_labels custom_prometheus_tags custom_prompt_dict custom_provider_map \
    darkbloom_models dashscope_models databricks_key databricks_models \
    datadog_llm_observability_params datadog_params datadog_use_v1 datarobot_key datarobot_models \
    deepgram_models deepinfra_models deepseek_models default_fallbacks default_in_memory_ttl \
    default_internal_user_params default_key_generate_params default_key_max_budget_alert_emails \
    default_max_internal_user_budget default_redis_batch_cache_expiry default_redis_ttl \
    default_soft_budget default_team_params default_team_settings delete_agent delete_eval \
    delete_responses delete_run delete_skill disable_add_prefix_to_prompt \
    disable_add_transform_inline_image_block disable_add_user_agent_to_request_tags \
    disable_aiohttp_transport disable_aiohttp_trust_env \
    disable_anthropic_gemini_context_caching_transform disable_copilot_system_to_assistant \
    disable_end_user_cost_tracking disable_end_user_cost_tracking_prometheus_only \
    disable_hf_tokenizer_download disable_stop_sequence_limit disable_streaming_logging \
    disable_token_counter disable_vertex_batch_output_transformation docker_model_runner_models \
    drop_params drop_params_env_flag dynamodb_table_name edenai_models elevenlabs_models email \
    empower_models enable_anthropic_prompt_caching enable_azure_ad_token_refresh \
    enable_caching_on_provider_specific_optional_params \
    enable_end_user_cost_tracking_prometheus_only enable_gemini_default_thinking_level_low \
    enable_json_schema_validation enable_key_alias_format_validation \
    enable_loadbalancing_on_batch_endpoints enable_model_config_credential_overrides \
    enable_preview_features error_logs expose_router_debug_in_errors extra_spend_tag_headers \
    failure_callback fal_ai_models fallbacks featherless_ai_models filter_invalid_headers \
    fireworks_ai_embedding_models fireworks_ai_models force_ipv4 \
    forward_traceparent_to_llm_provider friendliai_models galadriel_models gcs_pub_sub_use_v1 \
    gdc_api_base gdc_key gemini_live_defer_setup gemini_models generic_api_use_v1 \
    generic_logger_headers get_agent get_args get_eval get_litellm_gateway_api_key \
    get_model_cost_map get_responses get_run get_skill gigachat_key gigachat_models \
    github_copilot_models global_bitbucket_config global_disable_no_log_param global_gitlab_config \
    google_moderation_confidence_threshold gradient_ai_api_key gradient_ai_models groq_key \
    groq_models guardrail_name_config_map headers heroku_key heroku_models http2 huggingface_key \
    huggingface_models hyperbolic_models in_memory_llm_clients_cache inception_key inception_models \
    include_cost_in_streaming_usage infinity_key infinity_models initialized_langfuse_clients \
    input_callback interactions internal_user_budget_duration jina_ai_models json_logs \
    key_alias_pattern key_generation_settings known_tokenizer_config lambda_ai_models \
    langfuse_default_tags langfuse_enable_update_trace_keys langsmith_batch_size lemonade_key \
    lemonade_models list_agent_versions list_agents list_evals list_input_items list_runs \
    list_skills litellm_mode llama_api_key llama_models llamagate_models llamaguard_model_name \
    llamaguard_unsafe_content_categories llm_guard_mode llm_passthrough_route \
    log_auth_failure_key_identity log_client_error_tracebacks log_level log_raw_request_response \
    logged_real_time_event_types logging longer_context_model_fallback_dict maritalk_key \
    maritalk_models mark_litellm_import_complete max_budget max_end_user_budget \
    max_end_user_budget_id max_fallbacks max_internal_user_budget max_tokens max_ui_session_budget \
    max_user_budget mcp_tool_search minimax_models mistral_chat_models mock_responses_api_response \
    model_alias_map model_cost model_cost_map_url model_fallbacks model_group_settings model_list \
    model_list_set models_by_provider modelscope_models modify_params moonshot_models morph_models \
    nadir_models nebius_embedding_models nebius_key nebius_models network_mock newrelic_params \
    nlp_cloud_key nlp_cloud_models novita_api_key novita_models nscale_models num_retries \
    num_retries_per_request nvidia_nim_models nvidia_riva_models oci_models ollama_key \
    ollama_models open_ai_chat_completion_models open_ai_embedding_models \
    open_ai_text_completion_models openai_compatible_endpoints openai_compatible_providers \
    openai_image_generation_models openai_key openai_like_key openai_moderations_model_name \
    openai_system_messages_first openai_text_completion_compatible_providers \
    openai_video_generation_models openrouter_key openrouter_models organization \
    otel_tenant_destination_mode output_parse_pii overload overwrite_user_with_key_hash \
    ovhcloud_embedding_models ovhcloud_key ovhcloud_models palm_models perplexity_models \
    petals_models pointfive_params post_call_rules pre_call_rules predibase_key predibase_tenant_id \
    presidio_ad_hoc_recognizers priority_reservation project \
    prometheus_deployment_and_latency_caller_identity prometheus_emit_input_sequence_length_label \
    prometheus_emit_rate_limit_labels prometheus_emit_stream_label \
    prometheus_end_user_metrics_cleanup_interval_seconds \
    prometheus_end_user_metrics_max_series_per_metric prometheus_end_user_metrics_ttl_seconds \
    prometheus_exclude_labels prometheus_exclude_metrics prometheus_initialize_budget_metrics \
    prometheus_latency_buckets prometheus_metrics_config \
    prometheus_user_budget_label_include_email_alias prompt_name_config_map \
    provider_url_destination_allowed_hosts proxy_auth public_agent_groups \
    public_mcp_hub_strict_whitelist public_mcp_servers public_model_groups \
    public_model_groups_links public_skills_index publicai_models qwen_ai_platform_models \
    qwencloud_models rag reasoning_auto_summary recraft_models redact_messages_in_exceptions \
    redact_user_api_key_info reducto_models replicate_key replicate_models \
    request_correlation_in_logs request_timeout request_timeout_explicitly_set \
    require_auth_for_metrics_endpoint require_managed_files retry return_response_headers \
    route_all_chat_openai_to_responses run_server runwayml_models rust s3_audit_callback_params \
    s3_callback_params safe_memory_mode sambanova_embedding_models sambanova_models sap_service_key \
    secret_manager_client service_callback set_verbose skill_search_embedding_model \
    skip_system_message_in_guardrail skip_tool_message_in_guardrail snowflake_key snowflake_models \
    soniox_models sse_keepalive_ping_interval_seconds ssl_certificate ssl_ecdh_curve \
    ssl_security_level ssl_verify stability_models standard_logging_payload_excluded_fields \
    store_audit_logs strip_anthropic_total_tokens success_callback suppress_debug_info \
    tag_budget_config tencent_models text_completion_codestral_models \
    text_completion_inception_models together_ai_models togetherai_api_key token transcribe_models \
    turn_off_message_logging upperbound_key_generate_params use_aiohttp_transport \
    use_chat_completions_url_for_anthropic_messages use_client use_litellm_proxy \
    user_url_allowed_hosts user_url_validation v0_models validate_end_user_id_in_db \
    vector_store_file_content vector_store_file_create vector_store_file_delete \
    vector_store_file_list vector_store_file_retrieve vector_store_file_update \
    vector_store_index_registry vector_store_registry vector_store_search_failure_mode \
    verbose_logger vercel_ai_gateway_key vercel_ai_gateway_models vertexAITextEmbeddingConfig \
    vertex_ai_ai21_models vertex_ai_image_models vertex_ai_safety_settings vertex_ai_video_models \
    vertex_anthropic_models vertex_chat_models vertex_code_chat_models vertex_code_text_models \
    vertex_deepseek_models vertex_embedding_models vertex_language_models vertex_llama3_models \
    vertex_location vertex_minimax_models vertex_mistral_models vertex_moonshot_models \
    vertex_openai_models vertex_project vertex_text_models vertex_vision_models vertex_zai_models \
    volcengine_models voyage_models wandb_key wandb_models watsonx_models xai_key xai_models \
    zai_models zerobus_params";

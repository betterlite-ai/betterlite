pub(crate) const GENERAL_SETTINGS_KEYS: &str = "admission_queue_timeout_seconds alert_to_webhook_url alert_types alerting alerting_args \
    alerting_threshold allow_cli_sso_verification_uri_complete allow_unmanaged_response_ids \
    allowed_file_extensions allowed_routes apply_user_budget_to_team_keys \
    background_health_check_model_groups background_health_checks blocked_file_extensions \
    cancel_on_disconnect claude_code_gateway_managed_settings completion_model control_plane_url \
    coordination_redis custom_auth dangerously_permit_weak_or_unset_master_key database_args \
    database_connect_timeout database_connection_pool_limit database_connection_timeout \
    database_disable_prepared_statements database_extra_connection_params \
    database_max_idle_connection_lifetime database_socket_timeout database_type database_url \
    disable_auto_add_proxy_admin_to_teams disable_budget_reservation disable_env_credential_login \
    disable_password_login_when_sso_enabled disable_responses_id_security \
    enable_claude_code_gateway enable_openai_websocket_passthrough enable_public_model_hub \
    enforce_fallback_model_access fail_closed_rate_limit_enforcement failed_login_block_seconds \
    failed_login_window_seconds forward_client_headers_to_llm_api global_max_parallel_requests \
    health_check_concurrency health_check_interval health_check_skip_disabled_background_models \
    include_call_id_in_error_body infer_model_from_keys key_management_system master_key \
    max_batch_file_size_mb max_failed_login_attempts_per_source \
    max_failed_login_attempts_per_source_overrides max_file_size_mb \
    max_in_flight_requests_per_worker max_parallel_requests max_queued_requests_per_worker \
    max_request_size_mb max_response_size_mb maximum_autorouter_session_retention_period \
    maximum_daily_tag_spend_retention_period maximum_health_check_retention_period \
    maximum_spend_logs_cleanup_batch_size maximum_spend_logs_cleanup_batch_timeout \
    maximum_spend_logs_cleanup_max_batches maximum_spend_logs_cleanup_run_budget \
    maximum_spend_logs_retention_period mcp_advertised_versions mcp_allowed_clients \
    mcp_client_id_header mcp_internal_ip_ranges mcp_required_fields mcp_trusted_proxy_ranges \
    mcp_xff_num_trusted_hops missing_session_id model_list_healthy_only otel pass_through_endpoints \
    pass_through_request_timeout password_policy_min_length password_policy_require_lowercase \
    password_policy_require_numbers password_policy_require_special_characters \
    password_policy_require_uppercase plugins provider_url_destination_allowed_hosts \
    proxy_config_reload_interval_seconds reject_clientside_metadata_tags scheduled_job_stagger \
    spend_capture_rate_check store_model_in_db store_prompts_in_spend_logs supported_db_objects \
    transcribe_media_buckets trusted_proxy_ranges ui_access_mode use_azure_key_vault use_google_kms \
    use_spend_logs_partitioning user_api_key_cache_max_size user_header_mappings user_header_name \
    user_mcp_management_mode user_url_allowed_hosts user_url_validation";

pub(crate) const CACHE_PARAMS_KEYS: &str = "_backend azure_account_url azure_blob_container default_in_memory_ttl default_in_redis_ttl \
    disk_cache_dir gcp_service_account gcp_ssl_ca_certs gcs_bucket_name gcs_path \
    gcs_path_service_account host kwargs mode namespace password port qdrant_api_base \
    qdrant_api_key qdrant_collection_name qdrant_quantization_config \
    qdrant_semantic_cache_embedding_model qdrant_semantic_cache_vector_size redis_flush_size \
    redis_semantic_cache_embedding_model redis_semantic_cache_index_name redis_startup_nodes \
    s3_api_version s3_aws_access_key_id s3_aws_secret_access_key s3_aws_session_token \
    s3_bucket_name s3_config s3_endpoint_url s3_path s3_region_name s3_use_ssl s3_verify \
    semantic_cache_embedding_max_input_tokens semantic_cache_embedding_timeout semantic_cache_scope \
    similarity_threshold supported_call_types ttl type valkey_semantic_cache_embedding_model \
    valkey_semantic_cache_index_name";

pub(crate) const ROUTER_SETTINGS_KEYS: &str = "alerting_config allowed_fails allowed_fails_policy assistants_config \
    auto_router_capability_limit background_health_check_model_groups cache_kwargs cache_responses \
    caching_groups client_ttl content_policy_fallbacks context_window_fallbacks cooldown_time \
    debug_level default_fallbacks default_litellm_params default_max_parallel_requests \
    default_priority deployment_affinity_ttl_seconds disable_cooldowns enable_health_check_routing \
    enable_pre_call_checks enable_tag_filtering enable_weighted_failover fallback_access_check \
    fallback_budget_check fallbacks guardrail_list health_check_ignore_transient_errors \
    health_check_staleness_threshold ignore_invalid_deployments max_fallbacks \
    model_group_affinity_config model_group_alias model_group_retry_policy model_list num_retries \
    optional_pre_call_checks plugins polling_interval provider_budget_config redis_db redis_host \
    redis_password redis_port redis_url retry_after retry_policy router_general_settings \
    routing_groups routing_strategy routing_strategy_args search_tools set_verbose stream_timeout \
    tag_filtering_match_any tag_routing_prefix timeout";

pub(crate) const GUARDRAIL_KEYS: &str = "action additional_provider_specific_params advisory_system_message akto_account_id akto_api_key \
    akto_base_url akto_vxlan_id anonymize_input api_base api_endpoint api_id api_key api_version \
    application application_id assertions asset_id async_mode auth_token aws_access_key_id \
    aws_bedrock_runtime_endpoint aws_external_id aws_profile_name aws_region_name aws_role_name \
    aws_secret_access_key aws_session_name aws_session_token aws_sts_endpoint \
    aws_web_identity_token base_url block_failures block_on_error block_on_violation \
    blocked_languages blocked_words blocked_words_file breakdown categories category_thresholds \
    ccr_retrieval checks chunk_budget_chars client_id client_secret confidence_threshold config \
    content_filter_threshold content_moderation_check contextual_grounding_from_messages \
    credentials custom_code decision_model deepkeep_firewall_id default_action default_on \
    deployment_name detect_execution_intent detect_secrets_config detector_id detectors dev_info \
    disable_exception_on_block end_session_after_n_fails evaluation_id \
    experimental_use_latest_role_message_only extra_headers fail_on_error gateway_name \
    grounding_check grounding_strictness guard_name guardrail guardrailIdentifier guardrailVersion \
    guardrail_timeout hallucinations_check include_evidence include_scanners inspect_embeddings \
    is_detector_server keyword_redaction_tag lasso_conversation_id lasso_user_id location mask \
    mask_request_content mask_response_content max_concurrent_decision_calls max_input_chars \
    metadata mock_redacted_text mode model monitor_mode on_disallowed_action on_flagged \
    on_flagged_action on_sensitive_data on_violation only_scan_new_messages optional_params \
    output_parse_pii pangea_input_recipe pangea_output_recipe pattern_redaction_format patterns \
    payload persist_session pii_check pii_confidence_threshold pii_entities_config policy_id \
    policy_name policy_names post_checkpoint_id pre_checkpoint_id presidio_ad_hoc_recognizers \
    presidio_analyze_chunk_size_bytes presidio_analyzer_api_base presidio_anonymizer_api_base \
    presidio_entities_deny_list presidio_filter_scope presidio_language presidio_run_on \
    presidio_score_thresholds project_id prompt_attack_threshold prompt_injections \
    realtime_violation_message rules run_in_parallel sanitize_error_detail scan_only_tool_results \
    scan_raw_request send_user_api_key_alias send_user_api_key_team_id send_user_api_key_user_id \
    sensitive_data_route_to_model severity_threshold singulr_api_base singulr_api_key \
    singulr_application_id singulr_guardrail_id skip_system_message_in_guardrail \
    skip_tool_message_in_guardrail skip_unscannable_attachments sticky_session_routing template_id \
    tenant_id timeout tool_selection_quality_check tracker_api_base tracker_api_key \
    unreachable_fallback use_v2 verify_ssl version violation_message_template xecguard_model";

pub(crate) fn listed(names: &str, wanted: &str) -> bool {
    for name in names.split_ascii_whitespace() {
        if name == wanted {
            return true;
        }
    }
    false
}

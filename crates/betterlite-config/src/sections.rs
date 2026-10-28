use std::collections::BTreeSet;
use std::fmt;

use serde::de::DeserializeOwned;

use crate::loader::{ConfigError, EnvironmentLookup, parse_view};

pub trait Section: DeserializeOwned + Default {
    type Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup;
}

macro_rules! stub_section {
    () => {
        #[derive(Debug, Default, serde::Deserialize)]
        pub struct Raw {}

        #[derive(Debug, Clone, Default)]
        pub struct Settings;

        impl crate::sections::Section for Raw {
            type Settings = Settings;

            fn convert<L>(self, _lookup: &L) -> Result<Settings, crate::loader::ConfigError>
            where
                L: crate::loader::EnvironmentLookup,
            {
                Ok(Settings)
            }
        }
    };
}
pub(crate) use stub_section;

macro_rules! sections {
    ($($name:ident),* $(,)?) => {
        $(pub mod $name;)*

        #[derive(Clone, Default)]
        pub struct Sections {
            $(pub $name: $name::Settings,)*
        }

        // each section snapshots its own settings, so a new section never changes a shared snapshot
        impl fmt::Debug for Sections {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("Sections")
            }
        }

        pub(crate) struct RawSections {
            $($name: $name::Raw,)*
        }

        impl RawSections {
            pub(crate) fn parse(
                source: &str,
                ignored: &mut Vec<BTreeSet<String>>,
            ) -> Result<Self, ConfigError> {
                Ok(Self {
                    $($name: {
                        let (raw, paths) = parse_view(source)?;
                        ignored.push(paths);
                        raw
                    },)*
                })
            }

            pub(crate) fn convert<L>(self, lookup: &L) -> Result<Sections, ConfigError>
            where
                L: EnvironmentLookup,
            {
                Ok(Sections {
                    $($name: Section::convert(self.$name, lookup)?,)*
                })
            }
        }
    };
}

sections! {
    access_windows, adaptive_classification, admission_control, aim_inspection, akto_mirroring,
    alert_settings, anthropic_beta_config, anthropic_capability_catalog, anthropic_completion,
    anthropic_count_tokens, anthropic_credentials, anthropic_headers, anthropic_messages_routes,
    anthropic_option_primitives, anthropic_response, assistant_keepalive, assistant_settings,
    audio_routes, audit_events, auth_backend_dispatch, auth_cache, auth_cached_sessions,
    auth_common_checks, auth_context_sources, auth_end_users, auth_errors, auth_headers,
    auth_invalidation, auth_key_budgets, auth_key_validation, auth_limit_inputs,
    auth_model_budgets, auth_model_grants, auth_model_patterns, auth_org_lookup, auth_prefetch,
    auth_pricing, auth_reconnect, auth_request_policy, auth_route_patterns, auth_route_policy,
    auth_user_lookup, available_models, azure_arc_identity, azure_blob_cache, azure_client_secret,
    azure_credentials, azure_default_chain, azure_endpoints, azure_image_transport,
    azure_managed_identity, azure_oidc_exchange, azure_responses, azure_search,
    azure_transcription, azure_url_policy, batch_cost_finalize, batch_create_route,
    batch_limit_descriptors, batch_managed_records, batch_provider_cancel,
    batch_provider_create, batch_provider_list, batch_provider_read, batch_read_routes,
    batch_route_support, bedrock_agentcore_request, bedrock_aws_endpoints, bedrock_aws_settings,
    bedrock_converse_messages, bedrock_converse_options, bedrock_invoke_request,
    bedrock_legacy_models, bedrock_mantle_inputs, bedrock_model_capabilities, bedrock_nova_chat,
    bedrock_other_image_models, bedrock_other_titan_embeddings, bedrock_qwen_deepseek,
    bedrock_services_batch_create, bedrock_services_batch_records,
    bedrock_services_invoke_agent, bedrock_services_s3_access, bedrock_services_s3_signing,
    bedrock_services_search, bedrock_services_vector_stores, bedrock_settings,
    bedrock_web_identity, billing_metrics, budget_admission_handoff, budget_primitives,
    budget_reset, budget_windows, cache_cost_routing, capability_classifier,
    capability_forecast, catalog_updates, cato_lifecycle, cato_projections, central_auth_checks,
    classifier_context, classifier_message_text, claude_gateway_oauth, claude_gateway_policy,
    claude_platform_inputs, claude_skill_write, clickhouse_spend_logging, cloud_storage,
    complexity_adaptive_placement, complexity_config, complexity_placement,
    complexity_placement_gates, complexity_scoring, complexity_sessions,
    complexity_turn_overrides, component_contracts, compresr_writeback, content_category,
    content_filter_inputs, content_filter_records, context_placement, cooldown_decisions,
    cooldown_state, cost_routing, current_spend, custom_auth, custom_dimensions,
    database_availability, database_diagnostics, database_failures, database_reencryption,
    database_refresh_loop, database_startup, database_tls, database_tokens, database_urls,
    deepkeep_firewall, default_user_teams, deployment_affinity, deployment_catalog,
    deployment_clients, deployment_identity, dynamic_rate_limit_policy, end_user_identity,
    end_user_registry, enkryptai_detection, executed_batch_runner, fallback_plans,
    file_upload_options, file_upload_routing, file_upload_validation, focus_export_schedule,
    foundry_credentials, foundry_image_edits, foundry_vector_stores, full_model_metadata,
    gemini_cache_create, gemini_cache_prefix, generic_guardrail_api, grayswan, guardrail_identity,
    guardrail_judge, guardrail_translation, headroom_compression, headroom_hiddenlayer_support,
    health_probe_policy, hiddenlayer_evaluation, image_edit_call, image_generation_call,
    image_variations, includes, io_token_reservations, jev_classifier, jina_infinity_rerank,
    jwt_auth, jwt_claims, jwt_db_teams, jwt_key_resolution, jwt_keys, jwt_permissions,
    jwt_provisioning, jwt_role_claims, jwt_settings, jwt_team_authorization, jwt_team_selection,
    jwt_verification, key_access, key_context, key_create, key_event_sinks, key_limit_policy,
    key_persistence, key_policy_fields, key_rotation, key_rotation_write, lar1_routing,
    lasso_classification, latency_observations, latency_routing, least_busy,
    lemonade_model_metadata, llm_classifier, local_ttl_cache, login_form, login_return,
    login_routes, login_throttle, management_rbac, mcp_bearer_credentials, mcp_tool_helpers,
    mcp_toolset_storage, member_row_write, messages_advisor, messages_dispatch, messages_mcp,
    messages_native_call, messages_native_stream, messages_response_call,
    messages_responses_output, messages_thinking, migration_drift, migration_ledger,
    migration_recovery, model_create, model_effective_fields, model_generalizations,
    model_info_v1, model_management, model_patch, model_public_settings, model_rate_limits,
    model_visibility, oauth_tokens, ollama_model_metadata, openai_chat_content,
    openai_credentials, openai_mock_transport, openai_provider, openai_realtime,
    openai_workload_identity, pass_through_registry, passthrough_request_policies,
    password_login, password_policy, pre_routing_registry, prometheus_bounded_series,
    prompt_affinity, prompt_cache_accounting, proxy_custom_dispatch, proxy_provider_options,
    public_router_presets, quality_router, rag_records_and_chunking, rate_limit_usage,
    rds_credentials, redis_cache_primitives, redis_credentials, redis_failure_policy,
    request_body, request_body_safety, request_form, request_headers, request_intake,
    request_metadata_safety, request_model_overrides, request_records, request_scheduler,
    request_setup, request_setup_options, resource_sdk_settings, responses_id_security,
    responses_mcp_execute, responses_parameters, responses_ws_auth, responses_ws_native,
    retry_policy, router_compaction, router_core, router_weights, routing_budgets,
    routing_groups, routing_plugins, routing_state, routing_state_store, routing_timeouts,
    s3_response_cache, sandbox_dispatch, scheduled_job_stagger, semantic_router,
    sensitive_data_routing, session_iteration_limits, session_token_codec, settings,
    speech_call, spend_reseed, static_eligibility, summary_model_access, tag_matchers,
    tag_routing, team_admin_fields, team_create, team_fields, team_limits, team_state,
    tier_artifact_predictor, tier_definitions, transcription_call, trusted_network,
    two_solver_classifier, ui_session_cleanup, usage_routing, usage_routing_v2,
    usage_v2_accounting, user_delete, user_fields, user_info, user_update, verbose_hooks,
    vertex_credential_sources, vertex_embedding_dispatch, vertex_schema_shaping,
    virtual_key_auth, virtual_key_routes,
}

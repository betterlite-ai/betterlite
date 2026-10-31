pub mod access_windows;
pub mod adaptive_classification;
pub mod admission_control;
pub mod aim_inspection;
pub mod akto_mirroring;
pub mod alert_settings;
pub mod alice_conduct_decisions;
pub mod anthropic_beta_config;
pub mod anthropic_capability_catalog;
pub mod anthropic_completion;
pub mod anthropic_count_tokens;
pub mod anthropic_credentials;
pub mod anthropic_headers;
pub mod anthropic_messages_routes;
pub mod anthropic_option_primitives;
pub mod anthropic_response;
pub mod assistant_keepalive;
pub mod assistant_settings;
pub mod audio_routes;
pub mod audit_events;
pub mod auth_backend_dispatch;
pub mod auth_cache;
pub mod auth_cached_sessions;
pub mod auth_common_checks;
pub mod auth_context_sources;
pub mod auth_end_users;
pub mod auth_errors;
pub mod auth_headers;
pub mod auth_invalidation;
pub mod auth_key_budgets;
pub mod auth_key_validation;
pub mod auth_limit_inputs;
pub mod auth_model_budgets;
pub mod auth_model_grants;
pub mod auth_model_patterns;
pub mod auth_org_lookup;
pub mod auth_prefetch;
pub mod auth_pricing;
pub mod auth_reconnect;
pub mod auth_request_policy;
pub mod auth_route_patterns;
pub mod auth_route_policy;
pub mod auth_user_lookup;
pub mod available_models;
pub mod azure_arc_identity;
pub mod azure_blob_cache;
pub mod azure_client_secret;
pub mod azure_credentials;
pub mod azure_default_chain;
pub mod azure_endpoints;
pub mod azure_image_transport;
pub mod azure_managed_identity;
pub mod azure_oidc_exchange;
pub mod azure_responses;
pub mod azure_search;
pub mod azure_transcription;
pub mod azure_url_policy;
pub mod batch_cost_finalize;
pub mod batch_create_route;
pub mod batch_limit_descriptors;
pub mod batch_managed_records;
pub mod batch_provider_cancel;
pub mod batch_provider_create;
pub mod batch_provider_list;
pub mod batch_provider_read;
pub mod batch_read_routes;
pub mod batch_route_support;
pub mod bedrock_agentcore_request;
pub mod bedrock_aws_endpoints;
pub mod bedrock_aws_settings;
pub mod bedrock_converse_messages;
pub mod bedrock_converse_options;
pub mod bedrock_invoke_request;
pub mod bedrock_legacy_models;
pub mod bedrock_mantle_inputs;
pub mod bedrock_model_capabilities;
pub mod bedrock_nova_chat;
pub mod bedrock_other_image_models;
pub mod bedrock_other_titan_embeddings;
pub mod bedrock_qwen_deepseek;
pub mod bedrock_services_batch_create;
pub mod bedrock_services_batch_records;
pub mod bedrock_services_invoke_agent;
pub mod bedrock_services_s3_access;
pub mod bedrock_services_s3_signing;
pub mod bedrock_services_search;
pub mod bedrock_services_vector_stores;
pub mod bedrock_settings;
pub mod bedrock_web_identity;
pub mod billing_metrics;
pub mod budget_admission_handoff;
pub mod budget_primitives;
pub mod budget_reset;
pub mod budget_windows;
pub mod cache_cost_routing;
pub mod capability_classifier;
pub mod capability_forecast;
pub mod catalog_updates;
pub mod cato_lifecycle;
pub mod cato_projections;
pub mod central_auth_checks;
pub mod cisco_verdict_contracts;
pub mod classifier_context;
pub mod classifier_message_text;
pub mod claude_gateway_oauth;
pub mod claude_gateway_policy;
pub mod claude_platform_inputs;
pub mod claude_skill_write;
pub mod clickhouse_spend_logging;
pub mod cloud_storage;
pub mod complexity_adaptive_placement;
pub mod complexity_config;
pub mod complexity_placement;
pub mod complexity_placement_gates;
pub mod complexity_scoring;
pub mod complexity_sessions;
pub mod complexity_turn_overrides;
pub mod component_contracts;
pub mod compresr_writeback;
pub mod compression_interception_state;
pub mod content_category;
pub mod content_filter_catalog;
pub mod content_filter_inputs;
pub mod content_filter_records;
pub mod context_placement;
pub mod cooldown_decisions;
pub mod cooldown_state;
pub mod cost_routing;
pub mod current_spend;
pub mod custom_auth;
pub mod custom_dimensions;
pub mod database_availability;
pub mod database_diagnostics;
pub mod database_failures;
pub mod database_reencryption;
pub mod database_refresh_loop;
pub mod database_startup;
pub mod database_tls;
pub mod database_tokens;
pub mod database_urls;
pub mod deepkeep_firewall;
pub mod default_user_teams;
pub mod deployment_affinity;
pub mod deployment_catalog;
pub mod deployment_clients;
pub mod deployment_identity;
pub mod dynamic_rate_limit_policy;
pub mod dynamoai_moderation;
pub mod end_user_identity;
pub mod end_user_registry;
pub mod enkryptai_detection;
pub mod executed_batch_runner;
pub mod fallback_plans;
pub mod file_upload_options;
pub mod file_upload_routing;
pub mod file_upload_validation;
pub mod focus_export_schedule;
pub mod foundry_credentials;
pub mod foundry_image_edits;
pub mod foundry_vector_stores;
pub mod full_model_metadata;
pub mod gemini_cache_create;
pub mod gemini_cache_prefix;
pub mod generic_guardrail_api;
pub mod grayswan;
pub mod guardrail_identity;
pub mod guardrail_judge;
pub mod guardrail_translation;
pub mod headroom_compression;
pub mod headroom_hiddenlayer_support;
pub mod health_probe_policy;
pub mod hiddenlayer_evaluation;
pub mod ibm_guardrails;
pub mod image_edit_call;
pub mod image_generation_call;
pub mod image_variations;
pub mod includes;
pub mod io_token_reservations;
pub mod jev_classifier;
pub mod jina_infinity_rerank;
pub mod jwt_auth;
pub mod jwt_claims;
pub mod jwt_db_teams;
pub mod jwt_key_resolution;
pub mod jwt_keys;
pub mod jwt_permissions;
pub mod jwt_provisioning;
pub mod jwt_role_claims;
pub mod jwt_settings;
pub mod jwt_team_authorization;
pub mod jwt_team_selection;
pub mod jwt_verification;
pub mod key_access;
pub mod key_context;
pub mod key_create;
pub mod key_event_sinks;
pub mod key_limit_policy;
pub mod key_persistence;
pub mod key_policy_fields;
pub mod key_rotation;
pub mod key_rotation_write;
pub mod lar1_routing;
pub mod lasso_classification;
pub mod latency_observations;
pub mod latency_routing;
pub mod least_busy;
pub mod lemonade_model_metadata;
pub mod llm_classifier;
pub mod local_ttl_cache;
pub mod login_form;
pub mod login_return;
pub mod login_routes;
pub mod login_throttle;
pub mod management_rbac;
pub mod mcp_bearer_credentials;
pub mod mcp_tool_helpers;
pub mod mcp_toolset_storage;
pub mod member_row_write;
pub mod messages_advisor;
pub mod messages_dispatch;
pub mod messages_mcp;
pub mod messages_native_call;
pub mod messages_native_stream;
pub mod messages_response_call;
pub mod messages_responses_output;
pub mod messages_thinking;
pub mod migration_drift;
pub mod migration_ledger;
pub mod migration_recovery;
pub mod model_create;
pub mod model_effective_fields;
pub mod model_generalizations;
pub mod model_info_v1;
pub mod model_management;
pub mod model_patch;
pub mod model_public_settings;
pub mod model_rate_limits;
pub mod model_visibility;
pub mod oauth_tokens;
pub mod ollama_model_metadata;
pub mod openai_chat_content;
pub mod openai_credentials;
pub mod openai_mock_transport;
pub mod openai_provider;
pub mod openai_realtime;
pub mod openai_workload_identity;
pub mod otel_settings;
pub mod pass_through_registry;
pub mod passthrough_request_policies;
pub mod password_login;
pub mod password_policy;
pub mod pre_routing_registry;
pub mod prometheus_bounded_series;
pub mod prompt_affinity;
pub mod prompt_cache_accounting;
pub mod proxy_custom_dispatch;
pub mod proxy_provider_options;
pub mod public_router_presets;
pub mod quality_router;
pub mod rag_records_and_chunking;
pub mod rate_limit_usage;
pub mod rds_credentials;
pub mod redis_cache_primitives;
pub mod redis_credentials;
pub mod redis_failure_policy;
pub mod request_body;
pub mod request_body_safety;
pub mod request_form;
pub mod request_headers;
pub mod request_intake;
pub mod request_metadata_safety;
pub mod request_model_overrides;
pub mod request_records;
pub mod request_scheduler;
pub mod request_setup;
pub mod request_setup_options;
pub mod resource_sdk_settings;
pub mod responses_id_security;
pub mod responses_mcp_execute;
pub mod responses_parameters;
pub mod responses_ws_auth;
pub mod responses_ws_native;
pub mod retry_policy;
pub mod router_compaction;
pub mod router_core;
pub mod router_weights;
pub mod routing_budgets;
pub mod routing_groups;
pub mod routing_plugins;
pub mod routing_state;
pub mod routing_state_store;
pub mod routing_timeouts;
pub mod s3_response_cache;
pub mod sandbox_dispatch;
pub mod scheduled_job_stagger;
pub mod semantic_router;
pub mod sensitive_data_routing;
pub mod session_iteration_limits;
pub mod session_token_codec;
pub mod settings;
pub mod speech_call;
pub mod spend_reseed;
pub mod static_eligibility;
pub mod summary_model_access;
pub mod tag_matchers;
pub mod tag_routing;
pub mod team_admin_fields;
pub mod team_create;
pub mod team_fields;
pub mod team_limits;
pub mod team_state;
pub mod tier_artifact_predictor;
pub mod tier_definitions;
pub mod transcription_call;
pub mod trusted_network;
pub mod two_solver_classifier;
pub mod ui_session_cleanup;
pub mod usage_routing;
pub mod usage_routing_v2;
pub mod usage_v2_accounting;
pub mod user_delete;
pub mod user_fields;
pub mod user_info;
pub mod user_update;
pub mod verbose_hooks;
pub mod vertex_credential_sources;
pub mod vertex_embedding_dispatch;
pub mod vertex_schema_shaping;
pub mod virtual_key_auth;
pub mod virtual_key_routes;
pub mod websearch_request_hooks;
pub mod worker_options_reload;

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
    alert_settings, alice_conduct_decisions, anthropic_beta_config,
    anthropic_capability_catalog, anthropic_completion,
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
    capability_forecast, catalog_updates, cato_lifecycle,
    cato_projections, central_auth_checks, cisco_verdict_contracts,
    classifier_context, classifier_message_text, claude_gateway_oauth, claude_gateway_policy,
    claude_platform_inputs, claude_skill_write, clickhouse_spend_logging, cloud_storage,
    complexity_adaptive_placement, complexity_config, complexity_placement,
    complexity_placement_gates, complexity_scoring, complexity_sessions,
    complexity_turn_overrides, component_contracts, compresr_writeback,
    compression_interception_state, content_category, content_filter_catalog,
    content_filter_inputs, content_filter_records, context_placement, cost_adjustments,
    cooldown_decisions,
    cooldown_state, cost_routing, current_spend, custom_auth, custom_dimensions,
    database_availability, database_diagnostics, database_failures, database_reencryption,
    database_refresh_loop, database_startup, database_tls, database_tokens, database_urls,
    deepkeep_firewall, default_user_teams, deployment_affinity, deployment_catalog,
    deployment_clients, deployment_identity, dynamic_rate_limit_policy,
    dynamoai_moderation, end_user_identity,
    end_user_registry, enkryptai_detection, executed_batch_runner, fallback_plans,
    file_upload_options, file_upload_routing, file_upload_validation, focus_export_schedule,
    foundry_credentials, foundry_image_edits, foundry_vector_stores, full_model_metadata,
    gemini_cache_create, gemini_cache_prefix, generic_guardrail_api, grayswan, guardrail_identity,
    guardrail_judge, guardrail_translation, headroom_compression, headroom_hiddenlayer_support,
    health_probe_policy, hiddenlayer_evaluation, ibm_guardrails, image_edit_call,
    image_generation_call,
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
    openai_workload_identity, otel_settings, pass_through_registry, passthrough_request_policies,
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
    virtual_key_auth, virtual_key_routes, websearch_request_hooks, worker_options_reload,
}

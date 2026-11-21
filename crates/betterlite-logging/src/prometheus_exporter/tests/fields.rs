use rstest::rstest;

use crate::prometheus_exporter::{PrometheusCollectors, PrometheusExporterSettings};

use super::{default_label_settings, description, registered};

type CollectorName = fn(&PrometheusCollectors) -> String;

// one entry per PrometheusLogger attribute: the collector name its field must hold
const COLLECTOR_NAMES: [(&str, CollectorName); 81] = [
    ("litellm_proxy_failed_requests_metric", |collectors| {
        description(&collectors.litellm_proxy_failed_requests_metric)
            .name
            .clone()
    }),
    ("litellm_proxy_total_requests_metric", |collectors| {
        description(&collectors.litellm_proxy_total_requests_metric)
            .name
            .clone()
    }),
    ("litellm_request_total_latency_metric", |collectors| {
        description(&collectors.litellm_request_total_latency_metric)
            .name
            .clone()
    }),
    ("litellm_llm_api_latency_metric", |collectors| {
        description(&collectors.litellm_llm_api_latency_metric)
            .name
            .clone()
    }),
    ("litellm_llm_api_time_to_first_token_metric", |collectors| {
        description(&collectors.litellm_llm_api_time_to_first_token_metric)
            .name
            .clone()
    }),
    ("litellm_spend_metric", |collectors| {
        description(&collectors.litellm_spend_metric).name.clone()
    }),
    ("litellm_total_tokens_metric", |collectors| {
        description(&collectors.litellm_tokens_metric).name.clone()
    }),
    ("litellm_input_tokens_metric", |collectors| {
        description(&collectors.litellm_input_tokens_metric)
            .name
            .clone()
    }),
    ("litellm_output_tokens_metric", |collectors| {
        description(&collectors.litellm_output_tokens_metric)
            .name
            .clone()
    }),
    ("litellm_input_cached_tokens_metric", |collectors| {
        description(&collectors.litellm_input_cached_tokens_metric)
            .name
            .clone()
    }),
    ("litellm_input_cache_creation_tokens_metric", |collectors| {
        description(&collectors.litellm_input_cache_creation_tokens_metric)
            .name
            .clone()
    }),
    ("litellm_input_audio_tokens_metric", |collectors| {
        description(&collectors.litellm_input_audio_tokens_metric)
            .name
            .clone()
    }),
    ("litellm_output_reasoning_tokens_metric", |collectors| {
        description(&collectors.litellm_output_reasoning_tokens_metric)
            .name
            .clone()
    }),
    ("litellm_output_audio_tokens_metric", |collectors| {
        description(&collectors.litellm_output_audio_tokens_metric)
            .name
            .clone()
    }),
    ("litellm_video_duration_seconds_metric", |collectors| {
        description(&collectors.litellm_video_duration_seconds_metric)
            .name
            .clone()
    }),
    ("litellm_images_generated_metric", |collectors| {
        description(&collectors.litellm_images_generated_metric)
            .name
            .clone()
    }),
    ("litellm_remaining_team_budget_metric", |collectors| {
        description(&collectors.litellm_remaining_team_budget_metric)
            .name
            .clone()
    }),
    ("litellm_team_max_budget_metric", |collectors| {
        description(&collectors.litellm_team_max_budget_metric)
            .name
            .clone()
    }),
    ("litellm_team_budget_remaining_hours_metric", |collectors| {
        description(&collectors.litellm_team_budget_remaining_hours_metric)
            .name
            .clone()
    }),
    ("litellm_team_members_metric", |collectors| {
        description(&collectors.litellm_team_members_metric)
            .name
            .clone()
    }),
    ("litellm_remaining_org_budget_metric", |collectors| {
        description(&collectors.litellm_remaining_org_budget_metric)
            .name
            .clone()
    }),
    ("litellm_org_max_budget_metric", |collectors| {
        description(&collectors.litellm_org_max_budget_metric)
            .name
            .clone()
    }),
    ("litellm_org_budget_remaining_hours_metric", |collectors| {
        description(&collectors.litellm_org_budget_remaining_hours_metric)
            .name
            .clone()
    }),
    ("litellm_remaining_api_key_budget_metric", |collectors| {
        description(&collectors.litellm_remaining_api_key_budget_metric)
            .name
            .clone()
    }),
    ("litellm_api_key_max_budget_metric", |collectors| {
        description(&collectors.litellm_api_key_max_budget_metric)
            .name
            .clone()
    }),
    (
        "litellm_api_key_budget_remaining_hours_metric",
        |collectors| {
            description(&collectors.litellm_api_key_budget_remaining_hours_metric)
                .name
                .clone()
        },
    ),
    ("litellm_remaining_user_budget_metric", |collectors| {
        description(&collectors.litellm_remaining_user_budget_metric)
            .name
            .clone()
    }),
    ("litellm_user_max_budget_metric", |collectors| {
        description(&collectors.litellm_user_max_budget_metric)
            .name
            .clone()
    }),
    ("litellm_user_budget_remaining_hours_metric", |collectors| {
        description(&collectors.litellm_user_budget_remaining_hours_metric)
            .name
            .clone()
    }),
    ("litellm_remaining_customer_budget_metric", |collectors| {
        description(&collectors.litellm_remaining_customer_budget_metric)
            .name
            .clone()
    }),
    ("litellm_customer_max_budget_metric", |collectors| {
        description(&collectors.litellm_customer_max_budget_metric)
            .name
            .clone()
    }),
    (
        "litellm_customer_budget_remaining_hours_metric",
        |collectors| {
            description(&collectors.litellm_customer_budget_remaining_hours_metric)
                .name
                .clone()
        },
    ),
    (
        "litellm_remaining_api_key_requests_for_model",
        |collectors| {
            description(&collectors.litellm_remaining_api_key_requests_for_model)
                .name
                .clone()
        },
    ),
    ("litellm_remaining_api_key_tokens_for_model", |collectors| {
        description(&collectors.litellm_remaining_api_key_tokens_for_model)
            .name
            .clone()
    }),
    ("litellm_api_key_rate_limit_allowed_metric", |collectors| {
        description(&collectors.litellm_api_key_rate_limit_allowed_metric)
            .name
            .clone()
    }),
    ("litellm_api_key_rate_limit_used_metric", |collectors| {
        description(&collectors.litellm_api_key_rate_limit_used_metric)
            .name
            .clone()
    }),
    ("litellm_team_rate_limit_allowed_metric", |collectors| {
        description(&collectors.litellm_team_rate_limit_allowed_metric)
            .name
            .clone()
    }),
    ("litellm_team_rate_limit_used_metric", |collectors| {
        description(&collectors.litellm_team_rate_limit_used_metric)
            .name
            .clone()
    }),
    ("litellm_remaining_requests_metric", |collectors| {
        description(&collectors.litellm_remaining_requests_metric)
            .name
            .clone()
    }),
    ("litellm_remaining_tokens_metric", |collectors| {
        description(&collectors.litellm_remaining_tokens_metric)
            .name
            .clone()
    }),
    ("litellm_overhead_latency_metric", |collectors| {
        description(&collectors.litellm_overhead_latency_metric)
            .name
            .clone()
    }),
    (
        "litellm_overhead_with_guardrails_latency_metric",
        |collectors| {
            description(&collectors.litellm_overhead_with_guardrails_latency_metric)
                .name
                .clone()
        },
    ),
    ("litellm_request_queue_time_seconds", |collectors| {
        description(&collectors.litellm_request_queue_time_metric)
            .name
            .clone()
    }),
    ("litellm_guardrail_latency_seconds", |collectors| {
        description(&collectors.litellm_guardrail_latency_metric)
            .name
            .clone()
    }),
    ("litellm_guardrail_errors", |collectors| {
        description(&collectors.litellm_guardrail_errors_total)
            .name
            .clone()
    }),
    ("litellm_guardrail_requests", |collectors| {
        description(&collectors.litellm_guardrail_requests_total)
            .name
            .clone()
    }),
    ("litellm_provider_remaining_budget_metric", |collectors| {
        description(&collectors.litellm_provider_remaining_budget_metric)
            .name
            .clone()
    }),
    ("litellm_deployment_state", |collectors| {
        description(&collectors.litellm_deployment_state)
            .name
            .clone()
    }),
    ("litellm_deployment_tpm_limit", |collectors| {
        description(&collectors.litellm_deployment_tpm_limit)
            .name
            .clone()
    }),
    ("litellm_deployment_rpm_limit", |collectors| {
        description(&collectors.litellm_deployment_rpm_limit)
            .name
            .clone()
    }),
    ("litellm_deployment_cooled_down", |collectors| {
        description(&collectors.litellm_deployment_cooled_down)
            .name
            .clone()
    }),
    ("litellm_deployment_success_responses", |collectors| {
        description(&collectors.litellm_deployment_success_responses)
            .name
            .clone()
    }),
    ("litellm_deployment_failure_responses", |collectors| {
        description(&collectors.litellm_deployment_failure_responses)
            .name
            .clone()
    }),
    ("litellm_deployment_total_requests", |collectors| {
        description(&collectors.litellm_deployment_total_requests)
            .name
            .clone()
    }),
    (
        "litellm_deployment_latency_per_output_token",
        |collectors| {
            description(&collectors.litellm_deployment_latency_per_output_token)
                .name
                .clone()
        },
    ),
    ("litellm_deployment_successful_fallbacks", |collectors| {
        description(&collectors.litellm_deployment_successful_fallbacks)
            .name
            .clone()
    }),
    ("litellm_deployment_failed_fallbacks", |collectors| {
        description(&collectors.litellm_deployment_failed_fallbacks)
            .name
            .clone()
    }),
    ("litellm_callback_logging_failures_metric", |collectors| {
        description(&collectors.litellm_callback_logging_failures_metric)
            .name
            .clone()
    }),
    ("litellm_llm_api_failed_requests_metric", |collectors| {
        description(&collectors.litellm_llm_api_failed_requests_metric)
            .name
            .clone()
    }),
    ("litellm_requests_metric", |collectors| {
        description(&collectors.litellm_requests_metric)
            .name
            .clone()
    }),
    ("litellm_zero_cost_requests", |collectors| {
        description(&collectors.litellm_zero_cost_requests_total)
            .name
            .clone()
    }),
    ("litellm_spend_capture_rate", |collectors| {
        description(&collectors.litellm_spend_capture_rate)
            .name
            .clone()
    }),
    ("litellm_cache_hits_metric", |collectors| {
        description(&collectors.litellm_cache_hits_metric)
            .name
            .clone()
    }),
    ("litellm_cache_misses_metric", |collectors| {
        description(&collectors.litellm_cache_misses_metric)
            .name
            .clone()
    }),
    ("litellm_cached_tokens_metric", |collectors| {
        description(&collectors.litellm_cached_tokens_metric)
            .name
            .clone()
    }),
    (
        "litellm_provider_cache_read_input_tokens_metric",
        |collectors| {
            description(&collectors.litellm_provider_cache_read_input_tokens_metric)
                .name
                .clone()
        },
    ),
    (
        "litellm_provider_cache_creation_input_tokens_metric",
        |collectors| {
            description(&collectors.litellm_provider_cache_creation_input_tokens_metric)
                .name
                .clone()
        },
    ),
    ("litellm_total_users", |collectors| {
        description(&collectors.litellm_total_users_metric)
            .name
            .clone()
    }),
    ("litellm_active_users", |collectors| {
        description(&collectors.litellm_active_users_metric)
            .name
            .clone()
    }),
    ("litellm_teams_count", |collectors| {
        description(&collectors.litellm_teams_count_metric)
            .name
            .clone()
    }),
    ("litellm_managed_batch_created", |collectors| {
        description(&collectors.litellm_managed_batch_created_total)
            .name
            .clone()
    }),
    ("litellm_managed_file_size_bytes", |collectors| {
        description(&collectors.litellm_managed_file_size_bytes)
            .name
            .clone()
    }),
    ("litellm_managed_batch_duration_seconds", |collectors| {
        description(&collectors.litellm_managed_batch_duration_seconds)
            .name
            .clone()
    }),
    ("litellm_managed_file_created", |collectors| {
        description(&collectors.litellm_managed_file_created_total)
            .name
            .clone()
    }),
    ("litellm_managed_file_deleted", |collectors| {
        description(&collectors.litellm_managed_file_deleted_total)
            .name
            .clone()
    }),
    ("litellm_check_batch_cost_jobs_polled", |collectors| {
        description(&collectors.litellm_check_batch_cost_jobs_polled)
            .name
            .clone()
    }),
    ("litellm_check_batch_cost_jobs_processed", |collectors| {
        description(&collectors.litellm_check_batch_cost_jobs_processed_total)
            .name
            .clone()
    }),
    ("litellm_check_batch_cost_errors", |collectors| {
        description(&collectors.litellm_check_batch_cost_errors_total)
            .name
            .clone()
    }),
    (
        "litellm_check_batch_cost_last_run_timestamp",
        |collectors| {
            description(&collectors.litellm_check_batch_cost_last_run_timestamp)
                .name
                .clone()
        },
    ),
    ("litellm_mcp_tool_calls", |collectors| {
        description(&collectors.litellm_mcp_tool_calls_total)
            .name
            .clone()
    }),
    ("litellm_mcp_tool_call_spend_metric", |collectors| {
        description(&collectors.litellm_mcp_tool_call_spend_metric)
            .name
            .clone()
    }),
];

#[rstest]
#[case::litellm_proxy_failed_requests_metric("litellm_proxy_failed_requests_metric")]
#[case::litellm_proxy_total_requests_metric("litellm_proxy_total_requests_metric")]
#[case::litellm_request_total_latency_metric("litellm_request_total_latency_metric")]
#[case::litellm_llm_api_latency_metric("litellm_llm_api_latency_metric")]
#[case::litellm_llm_api_time_to_first_token_metric("litellm_llm_api_time_to_first_token_metric")]
#[case::litellm_spend_metric("litellm_spend_metric")]
#[case::litellm_total_tokens_metric("litellm_total_tokens_metric")]
#[case::litellm_input_tokens_metric("litellm_input_tokens_metric")]
#[case::litellm_output_tokens_metric("litellm_output_tokens_metric")]
#[case::litellm_input_cached_tokens_metric("litellm_input_cached_tokens_metric")]
#[case::litellm_input_cache_creation_tokens_metric("litellm_input_cache_creation_tokens_metric")]
#[case::litellm_input_audio_tokens_metric("litellm_input_audio_tokens_metric")]
#[case::litellm_output_reasoning_tokens_metric("litellm_output_reasoning_tokens_metric")]
#[case::litellm_output_audio_tokens_metric("litellm_output_audio_tokens_metric")]
#[case::litellm_video_duration_seconds_metric("litellm_video_duration_seconds_metric")]
#[case::litellm_images_generated_metric("litellm_images_generated_metric")]
#[case::litellm_remaining_team_budget_metric("litellm_remaining_team_budget_metric")]
#[case::litellm_team_max_budget_metric("litellm_team_max_budget_metric")]
#[case::litellm_team_budget_remaining_hours_metric("litellm_team_budget_remaining_hours_metric")]
#[case::litellm_team_members_metric("litellm_team_members_metric")]
#[case::litellm_remaining_org_budget_metric("litellm_remaining_org_budget_metric")]
#[case::litellm_org_max_budget_metric("litellm_org_max_budget_metric")]
#[case::litellm_org_budget_remaining_hours_metric("litellm_org_budget_remaining_hours_metric")]
#[case::litellm_remaining_api_key_budget_metric("litellm_remaining_api_key_budget_metric")]
#[case::litellm_api_key_max_budget_metric("litellm_api_key_max_budget_metric")]
#[case::litellm_api_key_budget_remaining_hours_metric(
    "litellm_api_key_budget_remaining_hours_metric"
)]
#[case::litellm_remaining_user_budget_metric("litellm_remaining_user_budget_metric")]
#[case::litellm_user_max_budget_metric("litellm_user_max_budget_metric")]
#[case::litellm_user_budget_remaining_hours_metric("litellm_user_budget_remaining_hours_metric")]
#[case::litellm_remaining_customer_budget_metric("litellm_remaining_customer_budget_metric")]
#[case::litellm_customer_max_budget_metric("litellm_customer_max_budget_metric")]
#[case::litellm_customer_budget_remaining_hours_metric(
    "litellm_customer_budget_remaining_hours_metric"
)]
#[case::litellm_remaining_api_key_requests_for_model(
    "litellm_remaining_api_key_requests_for_model"
)]
#[case::litellm_remaining_api_key_tokens_for_model("litellm_remaining_api_key_tokens_for_model")]
#[case::litellm_api_key_rate_limit_allowed_metric("litellm_api_key_rate_limit_allowed_metric")]
#[case::litellm_api_key_rate_limit_used_metric("litellm_api_key_rate_limit_used_metric")]
#[case::litellm_team_rate_limit_allowed_metric("litellm_team_rate_limit_allowed_metric")]
#[case::litellm_team_rate_limit_used_metric("litellm_team_rate_limit_used_metric")]
#[case::litellm_remaining_requests_metric("litellm_remaining_requests_metric")]
#[case::litellm_remaining_tokens_metric("litellm_remaining_tokens_metric")]
#[case::litellm_overhead_latency_metric("litellm_overhead_latency_metric")]
#[case::litellm_overhead_with_guardrails_latency_metric(
    "litellm_overhead_with_guardrails_latency_metric"
)]
#[case::litellm_request_queue_time_seconds("litellm_request_queue_time_seconds")]
#[case::litellm_guardrail_latency_seconds("litellm_guardrail_latency_seconds")]
#[case::litellm_guardrail_errors("litellm_guardrail_errors")]
#[case::litellm_guardrail_requests("litellm_guardrail_requests")]
#[case::litellm_provider_remaining_budget_metric("litellm_provider_remaining_budget_metric")]
#[case::litellm_deployment_state("litellm_deployment_state")]
#[case::litellm_deployment_tpm_limit("litellm_deployment_tpm_limit")]
#[case::litellm_deployment_rpm_limit("litellm_deployment_rpm_limit")]
#[case::litellm_deployment_cooled_down("litellm_deployment_cooled_down")]
#[case::litellm_deployment_success_responses("litellm_deployment_success_responses")]
#[case::litellm_deployment_failure_responses("litellm_deployment_failure_responses")]
#[case::litellm_deployment_total_requests("litellm_deployment_total_requests")]
#[case::litellm_deployment_latency_per_output_token("litellm_deployment_latency_per_output_token")]
#[case::litellm_deployment_successful_fallbacks("litellm_deployment_successful_fallbacks")]
#[case::litellm_deployment_failed_fallbacks("litellm_deployment_failed_fallbacks")]
#[case::litellm_callback_logging_failures_metric("litellm_callback_logging_failures_metric")]
#[case::litellm_llm_api_failed_requests_metric("litellm_llm_api_failed_requests_metric")]
#[case::litellm_requests_metric("litellm_requests_metric")]
#[case::litellm_zero_cost_requests("litellm_zero_cost_requests")]
#[case::litellm_spend_capture_rate("litellm_spend_capture_rate")]
#[case::litellm_cache_hits_metric("litellm_cache_hits_metric")]
#[case::litellm_cache_misses_metric("litellm_cache_misses_metric")]
#[case::litellm_cached_tokens_metric("litellm_cached_tokens_metric")]
#[case::litellm_provider_cache_read_input_tokens_metric(
    "litellm_provider_cache_read_input_tokens_metric"
)]
#[case::litellm_provider_cache_creation_input_tokens_metric(
    "litellm_provider_cache_creation_input_tokens_metric"
)]
#[case::litellm_total_users("litellm_total_users")]
#[case::litellm_active_users("litellm_active_users")]
#[case::litellm_teams_count("litellm_teams_count")]
#[case::litellm_managed_batch_created("litellm_managed_batch_created")]
#[case::litellm_managed_file_size_bytes("litellm_managed_file_size_bytes")]
#[case::litellm_managed_batch_duration_seconds("litellm_managed_batch_duration_seconds")]
#[case::litellm_managed_file_created("litellm_managed_file_created")]
#[case::litellm_managed_file_deleted("litellm_managed_file_deleted")]
#[case::litellm_check_batch_cost_jobs_polled("litellm_check_batch_cost_jobs_polled")]
#[case::litellm_check_batch_cost_jobs_processed("litellm_check_batch_cost_jobs_processed")]
#[case::litellm_check_batch_cost_errors("litellm_check_batch_cost_errors")]
#[case::litellm_check_batch_cost_last_run_timestamp("litellm_check_batch_cost_last_run_timestamp")]
#[case::litellm_mcp_tool_calls("litellm_mcp_tool_calls")]
#[case::litellm_mcp_tool_call_spend_metric("litellm_mcp_tool_call_spend_metric")]
fn every_field_holds_the_collector_named_for_its_attribute(#[case] expected: &str) {
    let (_registry, collectors) = registered(
        &PrometheusExporterSettings::default(),
        &default_label_settings(),
    );
    let (_, name_of) = COLLECTOR_NAMES
        .iter()
        .copied()
        .find(|(name, _)| *name == expected)
        .expect("the table names the collector");
    assert_eq!(name_of(&collectors), expected);
}

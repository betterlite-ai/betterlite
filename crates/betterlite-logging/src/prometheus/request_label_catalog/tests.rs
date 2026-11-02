#![cfg(test)]

use std::ptr;

use rstest::rstest;

use super::labels_for_metric;

#[test]
fn covers_exactly_thirty_four_request_and_deployment_metrics() {
    let covered = [
        "litellm_llm_api_latency_metric",
        "litellm_llm_api_time_to_first_token_metric",
        "litellm_request_total_latency_metric",
        "litellm_request_queue_time_seconds",
        "litellm_guardrail_latency_seconds",
        "litellm_guardrail_errors_total",
        "litellm_guardrail_requests_total",
        "litellm_proxy_total_requests_metric",
        "litellm_proxy_failed_requests_metric",
        "litellm_deployment_latency_per_output_token",
        "litellm_overhead_latency_metric",
        "litellm_overhead_with_guardrails_latency_metric",
        "litellm_remaining_requests_metric",
        "litellm_remaining_tokens_metric",
        "litellm_requests_metric",
        "litellm_spend_metric",
        "litellm_zero_cost_requests_total",
        "litellm_spend_capture_rate",
        "litellm_input_tokens_metric",
        "litellm_total_tokens_metric",
        "litellm_output_tokens_metric",
        "litellm_input_cached_tokens_metric",
        "litellm_input_cache_creation_tokens_metric",
        "litellm_input_audio_tokens_metric",
        "litellm_output_reasoning_tokens_metric",
        "litellm_output_audio_tokens_metric",
        "litellm_video_duration_seconds_metric",
        "litellm_images_generated_metric",
        "litellm_deployment_state",
        "litellm_deployment_tpm_limit",
        "litellm_deployment_rpm_limit",
        "litellm_deployment_cooled_down",
        "litellm_deployment_successful_fallbacks",
        "litellm_deployment_failed_fallbacks",
    ];
    assert_eq!(covered.len(), 34);
}

#[rstest]
#[case::api_latency("litellm_llm_api_latency_metric")]
#[case::time_to_first_token("litellm_llm_api_time_to_first_token_metric")]
#[case::request_total_latency("litellm_request_total_latency_metric")]
#[case::request_queue_time("litellm_request_queue_time_seconds")]
#[case::guardrail_latency("litellm_guardrail_latency_seconds")]
#[case::guardrail_errors("litellm_guardrail_errors_total")]
#[case::guardrail_requests("litellm_guardrail_requests_total")]
#[case::proxy_total_requests("litellm_proxy_total_requests_metric")]
#[case::proxy_failed_requests("litellm_proxy_failed_requests_metric")]
#[case::deployment_latency_per_output_token("litellm_deployment_latency_per_output_token")]
#[case::overhead_latency("litellm_overhead_latency_metric")]
#[case::overhead_with_guardrails_latency("litellm_overhead_with_guardrails_latency_metric")]
#[case::remaining_requests("litellm_remaining_requests_metric")]
#[case::remaining_tokens("litellm_remaining_tokens_metric")]
#[case::requests("litellm_requests_metric")]
#[case::spend("litellm_spend_metric")]
#[case::zero_cost_requests("litellm_zero_cost_requests_total")]
#[case::spend_capture_rate("litellm_spend_capture_rate")]
#[case::input_tokens("litellm_input_tokens_metric")]
#[case::total_tokens("litellm_total_tokens_metric")]
#[case::output_tokens("litellm_output_tokens_metric")]
#[case::input_cached_tokens("litellm_input_cached_tokens_metric")]
#[case::input_cache_creation_tokens("litellm_input_cache_creation_tokens_metric")]
#[case::input_audio_tokens("litellm_input_audio_tokens_metric")]
#[case::output_reasoning_tokens("litellm_output_reasoning_tokens_metric")]
#[case::output_audio_tokens("litellm_output_audio_tokens_metric")]
#[case::video_duration("litellm_video_duration_seconds_metric")]
#[case::images_generated("litellm_images_generated_metric")]
#[case::deployment_state("litellm_deployment_state")]
#[case::deployment_tpm_limit("litellm_deployment_tpm_limit")]
#[case::deployment_rpm_limit("litellm_deployment_rpm_limit")]
#[case::deployment_cooled_down("litellm_deployment_cooled_down")]
#[case::deployment_successful_fallbacks("litellm_deployment_successful_fallbacks")]
#[case::deployment_failed_fallbacks("litellm_deployment_failed_fallbacks")]
fn every_request_and_deployment_metric_has_a_label_schema(#[case] metric: &str) {
    assert!(labels_for_metric(metric).is_some());
}

#[test]
fn rejects_a_metric_outside_the_request_and_deployment_catalog() {
    assert_eq!(labels_for_metric("litellm_team_max_budget_metric"), None);
    assert_eq!(labels_for_metric("not_a_real_metric"), None);
}

#[test]
fn api_latency_and_time_to_first_token_carry_the_latency_label_set() {
    let expected: &[&str] = &[
        "model",
        "hashed_api_key",
        "api_key_alias",
        "team",
        "team_alias",
        "requested_model",
        "end_user",
        "user",
        "model_id",
        "api_provider",
        "service_tier",
    ];
    assert_eq!(
        labels_for_metric("litellm_llm_api_latency_metric").unwrap(),
        expected
    );
    assert_eq!(
        labels_for_metric("litellm_llm_api_time_to_first_token_metric").unwrap(),
        expected
    );
}

#[test]
fn request_total_latency_carries_service_tier_and_queue_time_does_not() {
    let total_latency: &[&str] = &[
        "end_user",
        "hashed_api_key",
        "api_key_alias",
        "requested_model",
        "team",
        "team_alias",
        "user",
        "model",
        "model_id",
        "api_provider",
        "service_tier",
    ];
    assert_eq!(
        labels_for_metric("litellm_request_total_latency_metric").unwrap(),
        total_latency
    );

    let queue_time: &[&str] = &[
        "end_user",
        "hashed_api_key",
        "api_key_alias",
        "requested_model",
        "team",
        "team_alias",
        "user",
        "model",
        "model_id",
        "api_provider",
    ];
    assert_eq!(
        labels_for_metric("litellm_request_queue_time_seconds").unwrap(),
        queue_time
    );
}

#[rstest]
#[case::input("litellm_input_tokens_metric")]
#[case::total("litellm_total_tokens_metric")]
#[case::output("litellm_output_tokens_metric")]
fn token_metrics_carry_user_email_and_no_client_ip_or_user_agent(#[case] metric: &str) {
    let expected: &[&str] = &[
        "end_user",
        "hashed_api_key",
        "api_key_alias",
        "model",
        "team",
        "team_alias",
        "user",
        "user_email",
        "requested_model",
        "model_id",
        "api_provider",
    ];
    assert_eq!(labels_for_metric(metric).unwrap(), expected);
}

#[test]
fn spend_metric_adds_service_tier_and_zero_cost_adds_the_reason_label() {
    let spend: &[&str] = &[
        "end_user",
        "hashed_api_key",
        "api_key_alias",
        "model",
        "team",
        "team_alias",
        "user",
        "user_email",
        "client_ip",
        "user_agent",
        "requested_model",
        "model_id",
        "api_provider",
        "service_tier",
    ];
    assert_eq!(labels_for_metric("litellm_spend_metric").unwrap(), spend);

    let zero_cost: &[&str] = &[
        "requested_model",
        "model",
        "model_id",
        "api_provider",
        "reason",
    ];
    assert_eq!(
        labels_for_metric("litellm_zero_cost_requests_total").unwrap(),
        zero_cost
    );

    assert_eq!(
        labels_for_metric("litellm_spend_capture_rate").unwrap(),
        &["api_provider"]
    );
}

#[test]
fn deployment_state_and_cooled_down_carry_model_id_api_base_and_api_provider() {
    let state: &[&str] = &["litellm_model_name", "model_id", "api_base", "api_provider"];
    assert_eq!(
        labels_for_metric("litellm_deployment_state").unwrap(),
        state
    );

    let cooled_down: &[&str] = &[
        "litellm_model_name",
        "model_id",
        "api_base",
        "api_provider",
        "exception_status",
    ];
    assert_eq!(
        labels_for_metric("litellm_deployment_cooled_down").unwrap(),
        cooled_down
    );
}

#[test]
fn tpm_and_rpm_limit_put_model_group_first_and_share_one_schema() {
    let expected: &[&str] = &[
        "model_group",
        "litellm_model_name",
        "model_id",
        "api_base",
        "api_provider",
    ];
    assert_eq!(
        labels_for_metric("litellm_deployment_tpm_limit").unwrap(),
        expected
    );
    assert_eq!(
        labels_for_metric("litellm_deployment_rpm_limit").unwrap(),
        expected
    );
    assert!(ptr::eq(
        labels_for_metric("litellm_deployment_tpm_limit").unwrap(),
        labels_for_metric("litellm_deployment_rpm_limit").unwrap(),
    ));
}

#[test]
fn fallback_metrics_carry_exception_status_and_class_not_status_code() {
    let expected: &[&str] = &[
        "requested_model",
        "fallback_model",
        "hashed_api_key",
        "api_key_alias",
        "team",
        "team_alias",
        "exception_status",
        "exception_class",
        "model_id",
    ];
    assert_eq!(
        labels_for_metric("litellm_deployment_successful_fallbacks").unwrap(),
        expected
    );
    assert_eq!(
        labels_for_metric("litellm_deployment_failed_fallbacks").unwrap(),
        expected
    );
    assert!(ptr::eq(
        labels_for_metric("litellm_deployment_successful_fallbacks").unwrap(),
        labels_for_metric("litellm_deployment_failed_fallbacks").unwrap(),
    ));
}

#[rstest]
#[case::input_cached("litellm_input_tokens_metric", "litellm_input_cached_tokens_metric")]
#[case::input_cache_creation(
    "litellm_input_tokens_metric",
    "litellm_input_cache_creation_tokens_metric"
)]
#[case::input_audio("litellm_input_tokens_metric", "litellm_input_audio_tokens_metric")]
#[case::output_reasoning(
    "litellm_output_tokens_metric",
    "litellm_output_reasoning_tokens_metric"
)]
#[case::output_audio("litellm_output_tokens_metric", "litellm_output_audio_tokens_metric")]
#[case::video_duration(
    "litellm_output_tokens_metric",
    "litellm_video_duration_seconds_metric"
)]
#[case::images_generated("litellm_output_tokens_metric", "litellm_images_generated_metric")]
fn token_detail_aliases_reference_their_base_metric_schema_by_identity(
    #[case] base: &str,
    #[case] alias: &str,
) {
    assert!(ptr::eq(
        labels_for_metric(base).unwrap(),
        labels_for_metric(alias).unwrap(),
    ));
}

#[rstest]
#[case::latency("litellm_guardrail_latency_seconds")]
#[case::errors("litellm_guardrail_errors_total")]
#[case::requests("litellm_guardrail_requests_total")]
fn guardrail_metrics_carry_no_labels_from_this_catalog(#[case] metric: &str) {
    let empty: &[&str] = &[];
    assert_eq!(labels_for_metric(metric).unwrap(), empty);
}

#[test]
fn proxy_total_requests_carries_status_code_route_and_client_identity() {
    let total: &[&str] = &[
        "end_user",
        "hashed_api_key",
        "api_key_alias",
        "requested_model",
        "team",
        "team_alias",
        "user",
        "status_code",
        "user_email",
        "route",
        "client_ip",
        "user_agent",
        "model_id",
        "api_provider",
    ];
    assert_eq!(
        labels_for_metric("litellm_proxy_total_requests_metric").unwrap(),
        total
    );
}

#[test]
fn proxy_failed_requests_carries_exception_fields_instead_of_status_code() {
    let failed: &[&str] = &[
        "end_user",
        "hashed_api_key",
        "api_key_alias",
        "requested_model",
        "team",
        "team_alias",
        "user",
        "user_email",
        "exception_status",
        "exception_class",
        "route",
        "client_ip",
        "user_agent",
        "model_id",
        "api_provider",
    ];
    assert_eq!(
        labels_for_metric("litellm_proxy_failed_requests_metric").unwrap(),
        failed
    );
}

#[test]
fn deployment_latency_overhead_and_remaining_metrics_carry_their_own_schemas() {
    let per_output_token: &[&str] = &[
        "litellm_model_name",
        "model_id",
        "api_base",
        "api_provider",
        "hashed_api_key",
        "api_key_alias",
        "team",
        "team_alias",
    ];
    assert_eq!(
        labels_for_metric("litellm_deployment_latency_per_output_token").unwrap(),
        per_output_token
    );

    let overhead: &[&str] = &[
        "model_group",
        "api_provider",
        "api_base",
        "litellm_model_name",
        "hashed_api_key",
        "api_key_alias",
        "model_id",
    ];
    assert_eq!(
        labels_for_metric("litellm_overhead_latency_metric").unwrap(),
        overhead
    );
    assert_eq!(
        labels_for_metric("litellm_overhead_with_guardrails_latency_metric").unwrap(),
        overhead
    );
    assert_eq!(
        labels_for_metric("litellm_remaining_requests_metric").unwrap(),
        overhead
    );
    assert_eq!(
        labels_for_metric("litellm_remaining_tokens_metric").unwrap(),
        overhead
    );
}

#[test]
fn requests_metric_carries_client_ip_and_user_agent_unlike_token_metrics() {
    let expected: &[&str] = &[
        "end_user",
        "hashed_api_key",
        "api_key_alias",
        "model",
        "team",
        "team_alias",
        "user",
        "user_email",
        "client_ip",
        "user_agent",
        "requested_model",
        "model_id",
        "api_provider",
    ];
    assert_eq!(
        labels_for_metric("litellm_requests_metric").unwrap(),
        expected
    );
}

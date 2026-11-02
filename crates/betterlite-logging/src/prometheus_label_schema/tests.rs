#![cfg(test)]

use rstest::rstest;

use super::{LabelSchemaError, PrometheusLabelSettings, get_labels, sanitize_label_name};
use betterlite_prometheus::caller_identity_logic::CallerIdentityError;

#[test]
fn config_defaults_match_litellm() {
    let settings = PrometheusLabelSettings::with_defaults();
    // prometheus_deployment_and_latency_caller_identity defaults to api_key_alias
    assert_eq!(settings.caller_identity, "api_key_alias");
    // prometheus_emit_stream_label defaults to false
    assert!(!settings.emit_stream_label);
    // prometheus_emit_rate_limit_labels defaults to false
    assert!(!settings.emit_rate_limit_labels);
    // prometheus_emit_input_sequence_length_label defaults to false
    assert!(!settings.budget_and_length.emit_input_sequence_length);
    // prometheus_user_budget_label_include_email_alias defaults to false
    assert!(!settings.budget_and_length.include_email_alias);
}

#[rstest]
#[case::team("litellm_remaining_team_budget_metric", &["team", "team_alias"])]
#[case::team_max("litellm_team_max_budget_metric", &["team", "team_alias"])]
#[case::team_members("litellm_team_members_metric", &["team", "team_alias"])]
#[case::org("litellm_remaining_org_budget_metric", &["org_id", "org_alias"])]
#[case::api_key("litellm_remaining_api_key_budget_metric", &["hashed_api_key", "api_key_alias"])]
#[case::user("litellm_remaining_user_budget_metric", &["user"])]
#[case::customer("litellm_remaining_customer_budget_metric", &["end_user"])]
fn budget_metrics_carry_their_schema(#[case] metric: &str, #[case] expected: &[&str]) {
    let settings = PrometheusLabelSettings::with_defaults();
    assert_eq!(get_labels(metric, &settings).unwrap(), expected);
}

#[rstest]
#[case::requests("litellm_remaining_api_key_requests_for_model")]
#[case::tokens("litellm_remaining_api_key_tokens_for_model")]
fn limit_for_model_metrics_carry_the_shared_schema(#[case] metric: &str) {
    let settings = PrometheusLabelSettings::with_defaults();
    let expected: &[&str] = &["hashed_api_key", "api_key_alias", "model", "model_id"];
    assert_eq!(get_labels(metric, &settings).unwrap(), expected);
}

#[test]
fn callback_logging_failures_carries_only_the_callback_name_label() {
    let settings = PrometheusLabelSettings::with_defaults();
    assert_eq!(
        get_labels("litellm_callback_logging_failures_metric", &settings).unwrap(),
        ["callback_name"]
    );
}

#[test]
fn llm_api_failed_requests_carries_the_full_identity_block() {
    let settings = PrometheusLabelSettings::with_defaults();
    let expected: &[&str] = &[
        "end_user",
        "hashed_api_key",
        "api_key_alias",
        "model",
        "team",
        "team_alias",
        "user",
        "model_id",
    ];
    assert_eq!(
        get_labels("litellm_llm_api_failed_requests_metric", &settings).unwrap(),
        expected
    );
}

#[rstest]
#[case::api_key_allowed("litellm_api_key_rate_limit_allowed_metric", &["hashed_api_key", "api_key_alias", "rate_limit_type"])]
#[case::api_key_used("litellm_api_key_rate_limit_used_metric", &["hashed_api_key", "api_key_alias", "rate_limit_type"])]
#[case::team_allowed("litellm_team_rate_limit_allowed_metric", &["team", "team_alias", "rate_limit_type"])]
#[case::team_used("litellm_team_rate_limit_used_metric", &["team", "team_alias", "rate_limit_type"])]
fn rate_limit_metrics_carry_only_rate_limit_type_by_default(
    #[case] metric: &str,
    #[case] expected: &[&str],
) {
    let settings = PrometheusLabelSettings::with_defaults();
    assert_eq!(get_labels(metric, &settings).unwrap(), expected);
}

#[test]
fn proxy_failed_requests_adds_rate_limit_category_when_toggled_on() {
    let mut settings = PrometheusLabelSettings::with_defaults();
    settings.emit_rate_limit_labels = true;
    let labels = get_labels("litellm_proxy_failed_requests_metric", &settings).unwrap();
    assert!(labels.contains(&"rate_limit_category".to_owned()));
    assert!(labels.contains(&"rate_limit_type".to_owned()));
}

#[rstest]
#[case::pod_lock("litellm_pod_lock_manager_size")]
#[case::daily_spend_memory("litellm_in_memory_daily_spend_update_queue_size")]
#[case::daily_spend_redis("litellm_redis_daily_spend_update_queue_size")]
#[case::spend_memory("litellm_in_memory_spend_update_queue_size")]
#[case::spend_redis("litellm_redis_spend_update_queue_size")]
#[case::file_size("litellm_managed_file_size_bytes")]
#[case::file_deleted("litellm_managed_file_deleted_total")]
#[case::jobs_polled("litellm_check_batch_cost_jobs_polled")]
#[case::errors("litellm_check_batch_cost_errors_total")]
#[case::last_run("litellm_check_batch_cost_last_run_timestamp")]
fn buffer_and_queue_metrics_carry_no_labels(#[case] metric: &str) {
    let settings = PrometheusLabelSettings::with_defaults();
    let empty: &[&str] = &[];
    assert_eq!(get_labels(metric, &settings).unwrap(), empty);
}

#[rstest]
#[case::hits("litellm_cache_hits_metric")]
#[case::misses("litellm_cache_misses_metric")]
#[case::cached_tokens("litellm_cached_tokens_metric")]
#[case::provider_read("litellm_provider_cache_read_input_tokens_metric")]
#[case::provider_creation("litellm_provider_cache_creation_input_tokens_metric")]
fn cache_metrics_share_the_nine_label_schema(#[case] metric: &str) {
    let settings = PrometheusLabelSettings::with_defaults();
    let expected: &[&str] = &[
        "model",
        "hashed_api_key",
        "api_key_alias",
        "team",
        "team_alias",
        "end_user",
        "user",
        "model_id",
        "api_provider",
    ];
    assert_eq!(get_labels(metric, &settings).unwrap(), expected);
}

#[rstest]
#[case::batch_created("litellm_managed_batch_created_total")]
#[case::file_created("litellm_managed_file_created_total")]
fn batch_user_metrics_share_the_five_label_schema(#[case] metric: &str) {
    let settings = PrometheusLabelSettings::with_defaults();
    let expected: &[&str] = &[
        "model",
        "api_provider",
        "user",
        "user_email",
        "api_key_alias",
    ];
    assert_eq!(get_labels(metric, &settings).unwrap(), expected);
}

#[rstest]
#[case::batch_duration("litellm_managed_batch_duration_seconds")]
#[case::jobs_processed("litellm_check_batch_cost_jobs_processed_total")]
fn batch_duration_metrics_carry_model_and_api_provider(#[case] metric: &str) {
    let settings = PrometheusLabelSettings::with_defaults();
    assert_eq!(
        get_labels(metric, &settings).unwrap(),
        ["model", "api_provider"]
    );
}

#[rstest]
#[case::tool_calls("litellm_mcp_tool_calls_total")]
#[case::spend("litellm_mcp_tool_call_spend_metric")]
fn mcp_metrics_carry_tool_and_caller_identity_labels(#[case] metric: &str) {
    let settings = PrometheusLabelSettings::with_defaults();
    let expected: &[&str] = &[
        "mcp_tool_name",
        "mcp_server_name",
        "hashed_api_key",
        "api_key_alias",
        "team",
        "team_alias",
        "user",
        "end_user",
    ];
    assert_eq!(get_labels(metric, &settings).unwrap(), expected);
}

#[rstest]
#[case::total("litellm_deployment_total_requests")]
#[case::success("litellm_deployment_success_responses")]
fn deployment_total_and_success_resolve_caller_identity(#[case] metric: &str) {
    let base: Vec<String> = [
        "model_group",
        "requested_model",
        "litellm_model_name",
        "model_id",
        "api_base",
        "api_provider",
        "hashed_api_key",
        "api_key_alias",
        "team",
        "team_alias",
        "client_ip",
        "user_agent",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    let mut settings = PrometheusLabelSettings::with_defaults();
    assert_eq!(get_labels(metric, &settings).unwrap(), base);

    settings.caller_identity = "user_email".to_owned();
    let user_email: Vec<String> = base
        .iter()
        .map(|label| {
            if label == "api_key_alias" {
                "user_email".to_owned()
            } else {
                label.clone()
            }
        })
        .collect();
    assert_eq!(get_labels(metric, &settings).unwrap(), user_email);

    settings.caller_identity = "both".to_owned();
    let both: Vec<String> = base
        .iter()
        .flat_map(|label| {
            if label == "api_key_alias" {
                vec![label.clone(), "user_email".to_owned()]
            } else {
                vec![label.clone()]
            }
        })
        .collect();
    assert_eq!(get_labels(metric, &settings).unwrap(), both);
}

#[test]
fn deployment_failure_resolves_caller_identity_with_exception_fields() {
    let metric = "litellm_deployment_failure_responses";
    let base: Vec<String> = [
        "model_group",
        "requested_model",
        "litellm_model_name",
        "model_id",
        "api_base",
        "api_provider",
        "exception_status",
        "exception_class",
        "hashed_api_key",
        "api_key_alias",
        "team",
        "team_alias",
        "client_ip",
        "user_agent",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    let mut settings = PrometheusLabelSettings::with_defaults();
    assert_eq!(get_labels(metric, &settings).unwrap(), base);

    settings.caller_identity = "both".to_owned();
    let both: Vec<String> = base
        .iter()
        .flat_map(|label| {
            if label == "api_key_alias" {
                vec![label.clone(), "user_email".to_owned()]
            } else {
                vec![label.clone()]
            }
        })
        .collect();
    assert_eq!(get_labels(metric, &settings).unwrap(), both);
}

#[test]
fn unknown_metric_is_rejected() {
    let settings = PrometheusLabelSettings::with_defaults();
    assert_eq!(
        get_labels("litellm_in_flight_requests", &settings),
        Err(LabelSchemaError::UnknownMetric {
            metric: "litellm_in_flight_requests".to_owned()
        })
    );
    assert_eq!(
        get_labels("not_a_real_metric", &settings),
        Err(LabelSchemaError::UnknownMetric {
            metric: "not_a_real_metric".to_owned()
        })
    );
}

#[test]
fn invalid_caller_identity_mode_is_rejected() {
    let mut settings = PrometheusLabelSettings::with_defaults();
    settings.caller_identity = "nonsense".to_owned();
    assert_eq!(
        get_labels("litellm_deployment_total_requests", &settings),
        Err(LabelSchemaError::CallerIdentity(
            CallerIdentityError::InvalidMode {
                value: "nonsense".to_owned()
            }
        ))
    );
}

#[test]
fn custom_metadata_and_tags_are_sanitized_and_appended() {
    let mut settings = PrometheusLabelSettings::with_defaults();
    settings.custom_metadata_labels = vec!["Request-ID".to_owned()];
    settings.custom_tags = vec!["beta-feature".to_owned()];
    let labels = get_labels("litellm_callback_logging_failures_metric", &settings).unwrap();
    assert!(labels.contains(&sanitize_label_name("Request-ID")));
    assert!(labels.contains(&sanitize_label_name("tag_beta-feature")));
}

#[test]
fn custom_metadata_label_duplicating_a_default_is_not_deduped() {
    let mut settings = PrometheusLabelSettings::with_defaults();
    settings.custom_metadata_labels = vec!["callback_name".to_owned()];
    let labels = get_labels("litellm_callback_logging_failures_metric", &settings).unwrap();
    assert_eq!(labels, ["callback_name", "callback_name"]);
}

#[test]
fn opt_in_toggle_label_already_present_in_custom_labels_is_not_duplicated() {
    let mut settings = PrometheusLabelSettings::with_defaults();
    settings.custom_metadata_labels = vec!["stream".to_owned()];
    settings.emit_stream_label = true;
    let labels = get_labels("litellm_proxy_total_requests_metric", &settings).unwrap();
    assert_eq!(labels.iter().filter(|label| *label == "stream").count(), 1);
}

#[test]
fn push_if_absent_skips_a_label_already_in_resolved_defaults() {
    let resolved = vec!["already_present".to_owned()];
    let mut custom_labels = Vec::new();
    super::push_if_absent(&resolved, &mut custom_labels, "already_present");
    assert!(custom_labels.is_empty());
    super::push_if_absent(&resolved, &mut custom_labels, "new_label");
    assert_eq!(custom_labels, ["new_label"]);
}

#[test]
fn org_label_is_injected_for_latency_metrics() {
    let settings = PrometheusLabelSettings::with_defaults();
    let labels = get_labels("litellm_llm_api_latency_metric", &settings).unwrap();
    assert!(labels.contains(&"org_id".to_owned()));
    assert!(labels.contains(&"org_alias".to_owned()));
}

#[test]
fn input_sequence_length_label_is_opt_in() {
    let mut settings = PrometheusLabelSettings::with_defaults();
    let metric = "litellm_llm_api_latency_metric";
    let without = get_labels(metric, &settings).unwrap();
    assert!(!without.contains(&"input_sequence_length".to_owned()));

    settings.budget_and_length.emit_input_sequence_length = true;
    let with = get_labels(metric, &settings).unwrap();
    assert!(with.contains(&"input_sequence_length".to_owned()));
}

#[test]
fn user_budget_email_alias_is_opt_in() {
    let mut settings = PrometheusLabelSettings::with_defaults();
    let metric = "litellm_user_max_budget_metric";
    let without = get_labels(metric, &settings).unwrap();
    assert!(!without.contains(&"user_email".to_owned()));

    settings.budget_and_length.include_email_alias = true;
    let with = get_labels(metric, &settings).unwrap();
    assert!(with.contains(&"user_email".to_owned()));
    assert!(with.contains(&"user_alias".to_owned()));
}

#[test]
fn proxy_total_requests_stream_label_is_opt_in() {
    let mut settings = PrometheusLabelSettings::with_defaults();
    let metric = "litellm_proxy_total_requests_metric";
    let without = get_labels(metric, &settings).unwrap();
    assert!(!without.contains(&"stream".to_owned()));

    settings.emit_stream_label = true;
    let with = get_labels(metric, &settings).unwrap();
    assert!(with.contains(&"stream".to_owned()));
}

#[test]
fn get_labels_result_is_independent_across_calls() {
    let settings = PrometheusLabelSettings::with_defaults();
    let mut first = get_labels("litellm_callback_logging_failures_metric", &settings).unwrap();
    first.push("extra".to_owned());
    assert_eq!(first, ["callback_name", "extra"]);
    let second = get_labels("litellm_callback_logging_failures_metric", &settings).unwrap();
    assert_eq!(second, ["callback_name"]);
}

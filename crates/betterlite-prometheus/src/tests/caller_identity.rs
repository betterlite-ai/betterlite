#![cfg(test)]
use betterlite_prometheus::caller_identity_logic::{
    CALLER_IDENTITY_METRICS, CallerIdentity, CallerIdentityError, resolve_caller_identity_labels,
    validate_caller_identity, validate_identity_settings,
};
use rstest::rstest;

#[rstest]
#[case::deployment("litellm_deployment_total_requests")]
#[case::success("litellm_deployment_success_responses")]
#[case::failure("litellm_deployment_failure_responses")]
#[case::total_latency("litellm_request_total_latency_metric")]
#[case::api_latency("litellm_llm_api_latency_metric")]
#[case::first_token("litellm_llm_api_time_to_first_token_metric")]
#[case::queue("litellm_request_queue_time_seconds")]
#[case::overhead("litellm_overhead_latency_metric")]
#[case::output_token("litellm_deployment_latency_per_output_token")]
fn resolves_modes(#[case] metric: &str) {
    assert!(CALLER_IDENTITY_METRICS.contains(&metric));
    let labels = vec![
        String::from("model"),
        String::from("api_key_alias"),
        String::from("team"),
    ];
    assert_eq!(
        resolve_caller_identity_labels(metric, &labels, None).unwrap(),
        labels
    );
    assert_eq!(
        resolve_caller_identity_labels(metric, &labels, Some("user_email")).unwrap(),
        ["model", "user_email", "team"]
    );
    assert_eq!(
        resolve_caller_identity_labels(metric, &labels, Some("both")).unwrap(),
        ["model", "api_key_alias", "user_email", "team"]
    );
    assert_eq!(
        resolve_caller_identity_labels(metric, &labels, Some("api_key_alias")).unwrap(),
        labels
    );
    assert!(matches!(
        resolve_caller_identity_labels(metric, &labels, Some("invalid")),
        Err(CallerIdentityError::InvalidMode { .. })
    ));
}

#[rstest]
#[case::empty("")]
#[case::case_sensitive("USER_EMAIL")]
#[case::whitespace("both ")]
fn rejects_invalid_mode(#[case] input: &str) {
    assert_eq!(
        validate_caller_identity(Some(input)),
        Err(CallerIdentityError::InvalidMode {
            value: input.to_owned()
        })
    );
}

#[test]
fn detects_conflicts() {
    let conflicts = vec![
        (
            String::from("litellm_deployment_total_requests"),
            vec![String::from("api_key_alias")],
        ),
        (
            String::from("litellm_llm_api_latency_metric"),
            vec![String::from("api_key_alias")],
        ),
    ];
    let error = validate_identity_settings(Some("user_email"), &conflicts).unwrap_err();
    assert_eq!(
        error,
        CallerIdentityError::ConflictingLabels {
            metrics: "litellm_deployment_total_requests, litellm_llm_api_latency_metric".to_owned()
        }
    );
    assert_eq!(
        validate_identity_settings(Some("both"), &conflicts).unwrap(),
        CallerIdentity::Both
    );
    assert_eq!(
        validate_identity_settings(None, &conflicts).unwrap(),
        CallerIdentity::ApiKeyAlias
    );
}

#[test]
fn preserves_unrelated_metrics() {
    let labels = vec!["api_key_alias".to_owned(), "user_email".to_owned()];
    assert_eq!(
        resolve_caller_identity_labels("litellm_spend_metric", &labels, Some("invalid")).unwrap(),
        labels
    );
    assert_eq!(
        validate_identity_settings(
            Some("user_email"),
            &[("litellm_spend_metric".to_owned(), labels)]
        )
        .unwrap(),
        CallerIdentity::UserEmail
    );
}

#[test]
fn accepts_email_configuration() {
    assert_eq!(
        validate_identity_settings(
            Some("user_email"),
            &[(
                "litellm_llm_api_latency_metric".to_owned(),
                vec!["user_email".to_owned()]
            )]
        )
        .unwrap(),
        CallerIdentity::UserEmail
    );
}

#[test]
fn reports_missing_alias() {
    assert!(matches!(
        resolve_caller_identity_labels("litellm_llm_api_latency_metric", &[], None),
        Err(CallerIdentityError::MissingAlias { .. })
    ));
}

#[test]
fn changes_only_first_alias() {
    let labels = vec!["api_key_alias".to_owned(), "api_key_alias".to_owned()];
    assert_eq!(
        resolve_caller_identity_labels(
            "litellm_llm_api_latency_metric",
            &labels,
            Some("user_email")
        )
        .unwrap(),
        ["user_email", "api_key_alias"]
    );
}

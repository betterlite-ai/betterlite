#![cfg(test)]
use betterlite_prometheus::validation::{
    LabelValidationError, MetricValidationError, ValidationResults, ValidationSummary,
};
use rstest::rstest;

#[rstest]
#[case::empty(false, false)]
#[case::metrics(true, false)]
#[case::labels(false, true)]
#[case::both(true, true)]
fn combines_messages(#[case] metric_error: bool, #[case] label_error: bool) {
    let mut results = ValidationResults {
        metric_errors: Vec::new(),
        label_errors: Vec::new(),
    };
    let mut expected = Vec::new();
    if metric_error {
        results.metric_errors.push(MetricValidationError {
            metric_name: "bad".to_owned(),
            valid_metrics: Vec::new(),
        });
        expected.push("Invalid metric name: bad".to_owned());
    }
    if label_error {
        results.label_errors.push(LabelValidationError {
            metric_name: "litellm_spend_metric".to_owned(),
            invalid_labels: vec!["bad".to_owned()],
            valid_labels: Vec::new(),
            caller_identity_mode: "both".to_owned(),
        });
        expected.push("Invalid labels for metric 'litellm_spend_metric': [\"bad\"]".to_owned());
    }
    assert_eq!(
        ValidationSummary::has_errors(&results),
        metric_error || label_error
    );
    assert_eq!(ValidationSummary::all_error_messages(&results), expected);
}

#[rstest]
#[case::alias("litellm_llm_api_latency_metric", "api_key_alias", true)]
#[case::email("litellm_llm_api_latency_metric", "user_email", true)]
#[case::other_label("litellm_llm_api_latency_metric", "bad", false)]
#[case::other_metric("litellm_spend_metric", "api_key_alias", false)]
fn caller_context(#[case] metric: &str, #[case] label: &str, #[case] context: bool) {
    let error = LabelValidationError {
        metric_name: metric.to_owned(),
        invalid_labels: vec![label.to_owned()],
        valid_labels: Vec::new(),
        caller_identity_mode: "user_email".to_owned(),
    };
    let base = format!("Invalid labels for metric '{metric}': [{label:?}]");
    let expected = if context {
        format!(
            "{base} (the caller-identity label on this metric is set by prometheus_deployment_and_latency_caller_identity=\"user_email\")"
        )
    } else {
        base
    };
    assert_eq!(error.message(), expected);
}

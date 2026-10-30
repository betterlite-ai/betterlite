use crate::validation::{LabelValidationError, MetricValidationError, ValidationResults};
#[test]
fn combines_messages() {
    let results = ValidationResults {
        metric_errors: vec![MetricValidationError {
            metric_name: String::from("bad"),
            valid_metrics: Vec::new(),
        }],
        label_errors: vec![LabelValidationError {
            metric_name: String::from("litellm_llm_api_latency_metric"),
            invalid_labels: vec![String::from("api_key_alias")],
            valid_labels: Vec::new(),
            caller_identity_mode: String::from("user_email"),
        }],
    };
    assert!(results.has_errors());
    assert_eq!(results.all_error_messages().len(), 2);
}

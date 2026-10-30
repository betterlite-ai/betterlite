use crate::caller_identity_logic::is_caller_identity_metric;
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MetricValidationError {
    pub metric_name: String,
    pub valid_metrics: Vec<String>,
}
impl MetricValidationError {
    #[must_use]
    pub fn message(&self) -> String {
        format!("Invalid metric name: {}", self.metric_name)
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LabelValidationError {
    pub metric_name: String,
    pub invalid_labels: Vec<String>,
    pub valid_labels: Vec<String>,
    pub caller_identity_mode: String,
}
impl LabelValidationError {
    #[must_use]
    pub fn message(&self) -> String {
        let base = format!(
            "Invalid labels for metric '{}': {:?}",
            self.metric_name, self.invalid_labels
        );
        if is_caller_identity_metric(&self.metric_name)
            && self
                .invalid_labels
                .iter()
                .any(|label| label == "api_key_alias" || label == "user_email")
        {
            format!(
                "{base} (the caller-identity label on this metric is set by prometheus_deployment_and_latency_caller_identity={:?})",
                self.caller_identity_mode
            )
        } else {
            base
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidationResults {
    pub metric_errors: Vec<MetricValidationError>,
    pub label_errors: Vec<LabelValidationError>,
}
impl ValidationResults {
    #[must_use]
    pub const fn has_errors(&self) -> bool {
        !self.metric_errors.is_empty() || !self.label_errors.is_empty()
    }
    #[must_use]
    pub fn all_error_messages(&self) -> Vec<String> {
        self.metric_errors
            .iter()
            .map(MetricValidationError::message)
            .chain(self.label_errors.iter().map(LabelValidationError::message))
            .collect()
    }
}

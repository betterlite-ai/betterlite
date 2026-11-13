use std::collections::{BTreeMap, BTreeSet};

use crate::label_names_catalog::UserApiKeyLabelName;
use crate::label_sanitization::sanitize_label_name;
use crate::validation::{ValidationResults, ValidationSummary as _};

pub type MetricSet = BTreeSet<String>;

// guardrail and service metrics carry these labels, and none is a UserAPIKeyLabelNames member
pub const NON_ENUM_METRIC_LABELS: [&str; 7] = [
    "guardrail_name",
    "status",
    "error_type",
    "hook_type",
    "purpose",
    "file_type",
    "result",
];

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PrometheusMetricsConfig {
    pub group: String,
    pub metrics: Vec<String>,
    pub include_labels: Vec<String>,
}

#[must_use]
pub fn all_defined_labels(metadata: &[String], tags: &[String]) -> MetricSet {
    let mut labels: MetricSet = UserApiKeyLabelName::ALL
        .iter()
        .map(|label| label.as_str().to_owned())
        .collect();
    labels.extend(
        NON_ENUM_METRIC_LABELS
            .iter()
            .map(|label| (*label).to_owned()),
    );
    labels.extend(metadata.iter().map(|label| sanitize_label_name(label)));
    labels.extend(
        tags.iter()
            .map(|tag| sanitize_label_name(&format!("tag_{tag}"))),
    );
    labels
}

#[must_use]
pub fn pretty_print_validation_errors(results: &ValidationResults) -> String {
    let mut report = String::from("Configuration validation errors:");
    for message in results.all_error_messages() {
        report.push_str("\n  ");
        report.push_str(&message);
    }
    report
}

#[must_use]
pub fn pretty_print_prometheus_config(
    enabled: &MetricSet,
    filters: &BTreeMap<String, Vec<String>>,
) -> String {
    let names = enabled
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(", ");
    let entries = filters
        .iter()
        .map(|(metric_name, labels)| format!("{metric_name}: {}", labels.join(", ")))
        .collect::<Vec<_>>()
        .join("; ");
    format!("Enabled metrics: {names}; label filters: {entries}")
}

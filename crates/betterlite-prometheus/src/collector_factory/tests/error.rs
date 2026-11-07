use crate::validation::{ValidationResults, ValidationSummary};

// the two exclude settings LiteLLM validates as sets, with the names that failed
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ExcludeValidation {
    pub invalid_metrics: Vec<String>,
    pub invalid_labels: Vec<String>,
}

impl ValidationSummary for ExcludeValidation {
    fn has_errors(&self) -> bool {
        !self.invalid_metrics.is_empty() || !self.invalid_labels.is_empty()
    }

    fn all_error_messages(&self) -> Vec<String> {
        self.invalid_metrics
            .iter()
            .map(|metric| format!("Invalid metric name in prometheus_exclude_metrics: {metric}"))
            .chain(
                self.invalid_labels.iter().map(|label| {
                    format!("Invalid label name in prometheus_exclude_labels: {label}")
                }),
            )
            .collect()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CollectorFactoryError<E> {
    #[error("Configuration validation failed:\n{}", .results.all_error_messages().join("\n"))]
    Configuration { results: ValidationResults },
    #[error(
        "Prometheus exclude configuration validation failed:\n{}",
        .invalid.all_error_messages().join("\n")
    )]
    Exclusions { invalid: ExcludeValidation },
    #[error(transparent)]
    LabelSchema(#[from] E),
}

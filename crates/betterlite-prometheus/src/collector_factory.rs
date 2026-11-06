use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;

use rust_decimal::Decimal;

use crate::label_names_catalog::UserApiKeyLabelName;
use crate::label_sanitization::sanitize_label_name;
use crate::metric_catalog::DEFINED_PROMETHEUS_METRICS;
use crate::no_op_metric::NoOpMetric;
use crate::validation::{
    LabelValidationError, MetricValidationError, ValidationResults, ValidationSummary as _,
};

mod tests;

pub type MetricSet = BTreeSet<String>;
// guardrail and service metrics carry these labels, and none is a UserAPIKeyLabelNames member
#[rustfmt::skip]
pub const NON_ENUM_METRIC_LABELS: [&str; 7] = ["guardrail_name", "status", "error_type", "hook_type", "purpose", "file_type", "result"];
// decimal keeps a money increment exact; inf and NaN stay explicit instead of becoming a float
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[rustfmt::skip]
pub enum MetricValue { Finite(Decimal), PositiveInfinity, NegativeInfinity, NotANumber }
// one update request, so a wrapper forwards every metric operation through a single method
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[rustfmt::skip]
pub enum MetricUpdate { Inc(MetricValue), Set(MetricValue), Observe(MetricValue), Remove }
// the update is the one method a wrapper forwards; the rest are the native collector interface
#[rustfmt::skip]
pub trait Collector: Sized + Clone {
    #[must_use]
    fn labels(&self, labels: &[(&str, &str)]) -> Self;
    fn update(&self, update: MetricUpdate);
    fn inc(&self, value: MetricValue) { self.update(MetricUpdate::Inc(value)); }
    fn set(&self, value: MetricValue) { self.update(MetricUpdate::Set(value)); }
    fn observe(&self, value: MetricValue) { self.update(MetricUpdate::Observe(value)); }
    fn remove(&self) { self.update(MetricUpdate::Remove); }
}
#[rustfmt::skip]
impl Collector for NoOpMetric {
    fn labels(&self, _labels: &[(&str, &str)]) -> Self { *self }
    fn update(&self, _update: MetricUpdate) {}
}
// a metric registered without the globally excluded labels: every labels(...) call drops them too
#[derive(Clone, Debug)]
#[rustfmt::skip]
pub struct ExcludedLabelMetric<M> { pub metric: M, pub excluded_labels: MetricSet }
#[rustfmt::skip]
impl<M: Collector> Collector for ExcludedLabelMetric<M> {
    fn labels(&self, labels: &[(&str, &str)]) -> Self {
        let kept = labels.iter().copied().filter(|(name, _)| !self.excluded_labels.contains(*name)).collect::<Vec<_>>();
        if kept.is_empty() { return self.clone(); }
        Self { metric: self.metric.labels(&kept), excluded_labels: self.excluded_labels.clone() }
    }
    fn update(&self, update: MetricUpdate) { self.metric.update(update); }
}
// LiteLLM's _MetricLike: a disabled metric, a label-filtered metric, or the registered one
#[derive(Clone, Debug)]
#[rustfmt::skip]
pub enum MetricLike<M> { NoOp(NoOpMetric), Excluded(ExcludedLabelMetric<M>), Real(M) }
impl<M: Collector> Collector for MetricLike<M> {
    fn labels(&self, labels: &[(&str, &str)]) -> Self {
        match self {
            Self::NoOp(metric) => Self::NoOp(metric.labels(labels)),
            Self::Excluded(metric) => Self::Excluded(metric.labels(labels)),
            Self::Real(metric) => Self::Real(metric.labels(labels)),
        }
    }
    fn update(&self, update: MetricUpdate) {
        match self {
            Self::NoOp(metric) => metric.update(update),
            Self::Excluded(metric) => metric.update(update),
            Self::Real(metric) => metric.update(update),
        }
    }
}
#[derive(Clone, Debug, Default, Eq, PartialEq)]
#[rustfmt::skip]
pub struct PrometheusMetricsConfig { pub group: String, pub metrics: Vec<String>, pub include_labels: Vec<String> }
// the label schema lives in betterlite-logging, which depends on this crate, so it comes in here
pub trait LabelSchema {
    type Error: Error + 'static;
    fn labels(&self, metric: &str) -> Result<Vec<String>, Self::Error>;
    fn caller_identity_mode(&self) -> &str;
}
#[derive(Debug, thiserror::Error)]
pub enum CollectorFactoryError<E> {
    #[error("Configuration validation failed:\n{}", .messages.join("\n"))]
    Configuration { messages: Vec<String> },
    #[error("Prometheus exclude configuration validation failed:\n{}", .messages.join("\n"))]
    Exclusions { messages: Vec<String> },
    #[error(transparent)]
    LabelSchema(#[from] E),
}
pub struct CollectorFactory<S> {
    pub schema: S,
    pub enabled_metrics: Option<MetricSet>,
    pub exclude_metrics: MetricSet,
    pub exclude_labels: MetricSet,
    pub label_filters: BTreeMap<String, Vec<String>>,
    pub cached_labels: BTreeMap<String, Vec<String>>,
}
// LiteLLM enables every metric unless an explicit include list or exclude list says otherwise
#[must_use]
pub fn is_metric_enabled(enabled: &Option<MetricSet>, excluded: &MetricSet, metric: &str) -> bool {
    if excluded.contains(metric) {
        return false;
    }
    enabled
        .as_ref()
        .is_none_or(|metrics| metrics.is_empty() || metrics.contains(metric))
}
impl<S: LabelSchema> CollectorFactory<S> {
    #[must_use]
    pub const fn new(schema: S) -> Self {
        Self {
            schema,
            enabled_metrics: None,
            exclude_metrics: MetricSet::new(),
            exclude_labels: MetricSet::new(),
            label_filters: BTreeMap::new(),
            cached_labels: BTreeMap::new(),
        }
    }

    pub fn parse_prometheus_config(
        &mut self,
        configs: &[PrometheusMetricsConfig],
    ) -> Result<(), CollectorFactoryError<S::Error>> {
        if configs.is_empty() {
            return Ok(());
        }
        let mut found = ValidationResults {
            metric_errors: vec![],
            label_errors: vec![],
        };
        let mut filters = BTreeMap::new();
        let mut enabled = MetricSet::new();
        let metrics = configs
            .iter()
            .flat_map(|config| config.metrics.iter().map(move |metric| (metric, config)));
        for (metric_name, config) in metrics {
            enabled.insert(metric_name.clone());
            self.validate_metric(
                metric_name,
                &config.include_labels,
                &mut filters,
                &mut found,
            )?;
        }
        self.enabled_metrics = Some(enabled);
        if found.has_errors() {
            let messages = found.all_error_messages();
            tracing::error!("{}", pretty_print_validation_errors(&found));
            return Err(CollectorFactoryError::Configuration { messages });
        }
        self.label_filters = filters;
        let summary = pretty_print_prometheus_config(&self.enabled_metrics, &self.label_filters);
        tracing::info!("{summary}");
        Ok(())
    }

    fn validate_metric(
        &self,
        metric_name: &str,
        include_labels: &[String],
        filters: &mut BTreeMap<String, Vec<String>>,
        found: &mut ValidationResults,
    ) -> Result<(), CollectorFactoryError<S::Error>> {
        if !DEFINED_PROMETHEUS_METRICS.contains(&metric_name) {
            let valid = DEFINED_PROMETHEUS_METRICS
                .iter()
                .map(|name| (*name).to_owned())
                .collect();
            found.metric_errors.push(MetricValidationError {
                metric_name: metric_name.to_owned(),
                valid_metrics: valid,
            });
            return Ok(());
        }
        if include_labels.is_empty() {
            return Ok(());
        }
        let valid = self.schema.labels(metric_name)?;
        let invalid: Vec<String> = include_labels
            .iter()
            .filter(|label| !valid.contains(label))
            .cloned()
            .collect();
        if invalid.is_empty() {
            filters.insert(metric_name.to_owned(), include_labels.to_vec());
            return Ok(());
        }
        found.label_errors.push(LabelValidationError {
            metric_name: metric_name.to_owned(),
            invalid_labels: invalid,
            valid_labels: valid,
            caller_identity_mode: self.schema.caller_identity_mode().to_owned(),
        });
        Ok(())
    }

    #[rustfmt::skip]
    pub fn parse_exclude_config(
        &mut self,
        exclude_metrics: &[String],
        exclude_labels: &[String],
        custom_metadata_labels: &[String],
        custom_tags: &[String],
    ) -> Result<(), CollectorFactoryError<S::Error>> {
        let metrics: BTreeSet<&str> = exclude_metrics.iter().map(String::as_str).collect();
        let labels: BTreeSet<&str> = exclude_labels.iter().map(String::as_str).collect();
        let defined = all_defined_labels(custom_metadata_labels, custom_tags);
        let invalid_metrics = metrics.iter().filter(|metric| !DEFINED_PROMETHEUS_METRICS.contains(*metric));
        let invalid_labels = labels.iter().filter(|label| !defined.contains(**label));
        let messages = invalid_metrics
            .map(|metric| format!("Invalid metric name in prometheus_exclude_metrics: {metric}"))
            .chain(invalid_labels.map(|label| format!("Invalid label name in prometheus_exclude_labels: {label}")))
            .collect::<Vec<_>>();
        if !messages.is_empty() {
            return Err(CollectorFactoryError::Exclusions { messages });
        }
        self.exclude_metrics = metrics.iter().map(|metric| (*metric).to_owned()).collect();
        self.exclude_labels = labels.iter().map(|label| (*label).to_owned()).collect();
        Ok(())
    }

    pub fn get_labels_for_metric(
        &mut self,
        metric_name: &str,
    ) -> Result<Vec<String>, CollectorFactoryError<S::Error>> {
        if let Some(cached) = self.cached_labels.get(metric_name) {
            return Ok(cached.clone());
        }
        let defaults = self.schema.labels(metric_name)?;
        let filter = self.label_filters.get(metric_name);
        let resolved = defaults
            .into_iter()
            .filter(|label| !self.exclude_labels.contains(label))
            .filter(|label| filter.is_none_or(|allowed| allowed.contains(label)))
            .collect::<Vec<_>>();
        self.cached_labels
            .insert(metric_name.to_owned(), resolved.clone());
        Ok(resolved)
    }

    #[rustfmt::skip]
    pub fn create_metric_factory<M, C>(
        &self,
        construct: C,
    ) -> impl Fn(&str, &[&str]) -> MetricLike<M>
    where
        M: Collector,
        C: Fn(&str, &[&str]) -> M,
    {
        let enabled = self.enabled_metrics.clone();
        let excluded_metrics = self.exclude_metrics.clone();
        let excluded_labels = self.exclude_labels.clone();
        move |name, labelnames| {
            if !is_metric_enabled(&enabled, &excluded_metrics, name) {
                return MetricLike::NoOp(NoOpMetric);
            }
            let kept = labelnames.iter().copied().filter(|label| !excluded_labels.contains(*label)).collect::<Vec<_>>();
            if kept.len() == labelnames.len() {
                return MetricLike::Real(construct(name, labelnames));
            }
            MetricLike::Excluded(ExcludedLabelMetric { metric: construct(name, &kept), excluded_labels: excluded_labels.clone() })
        }
    }
}
#[must_use]
#[rustfmt::skip]
pub fn all_defined_labels(metadata: &[String], tags: &[String]) -> MetricSet {
    let mut labels: MetricSet = UserApiKeyLabelName::ALL.iter().map(|label| label.as_str().to_owned()).collect();
    labels.extend(NON_ENUM_METRIC_LABELS.iter().map(|label| (*label).to_owned()));
    labels.extend(metadata.iter().map(|label| sanitize_label_name(label)));
    labels.extend(tags.iter().map(|tag| sanitize_label_name(&format!("tag_{tag}"))));
    labels
}
#[must_use]
pub fn pretty_print_validation_errors(results: &ValidationResults) -> String {
    let mut report = String::from("Configuration validation errors:");
    report.push_str(&results.all_error_messages().join("\n"));
    report
}
#[must_use]
#[rustfmt::skip]
pub fn pretty_print_prometheus_config(enabled: &Option<MetricSet>, filters: &BTreeMap<String, Vec<String>>) -> String {
    let names = enabled.iter().flatten().map(String::as_str).collect::<Vec<_>>().join(", ");
    format!("Enabled metrics: {names}; label filters: {filters:?}")
}
// the callback list from B189 viewed by slot: each slot holds the logger when it is one
#[must_use]
#[rustfmt::skip]
pub fn get_instance<'callbacks, L>(
    callbacks: &'callbacks [Option<&'callbacks L>],
) -> Option<&'callbacks L> {
    callbacks
        .iter()
        .find_map(|callback| *callback)
}

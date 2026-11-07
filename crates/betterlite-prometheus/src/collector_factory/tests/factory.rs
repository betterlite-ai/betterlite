use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;

use crate::collector_factory::collector::{Collector, ExcludedLabelMetric, MetricLike};
use crate::collector_factory::config::{
    MetricSet, PrometheusMetricsConfig, all_defined_labels, pretty_print_prometheus_config,
    pretty_print_validation_errors,
};
use crate::collector_factory::error::{CollectorFactoryError, ExcludeValidation};
use crate::metric_catalog::DEFINED_PROMETHEUS_METRICS;
use crate::no_op_metric::NoOpMetric;
use crate::validation::{
    LabelValidationError, MetricValidationError, ValidationResults, ValidationSummary as _,
};

// the label schema lives in betterlite-logging, which depends on this crate, so it comes in here
pub trait LabelSchema {
    type Error: Error + 'static;
    fn labels(&self, metric: &str) -> Result<Vec<String>, Self::Error>;
    fn caller_identity_mode(&self) -> &str;
}

pub struct CollectorFactory<S> {
    pub schema: S,
    pub enabled_metrics: MetricSet,
    pub exclude_metrics: MetricSet,
    pub exclude_labels: MetricSet,
    pub label_filters: BTreeMap<String, Vec<String>>,
    pub cached_labels: BTreeMap<String, Vec<String>>,
}

// LiteLLM enables every metric unless an explicit include list or exclude list says otherwise
#[must_use]
pub fn is_metric_enabled(enabled: &MetricSet, excluded: &MetricSet, metric: &str) -> bool {
    if excluded.contains(metric) {
        return false;
    }
    enabled.is_empty() || enabled.contains(metric)
}

impl<S: LabelSchema> CollectorFactory<S> {
    #[must_use]
    pub const fn new(schema: S) -> Self {
        Self {
            schema,
            enabled_metrics: MetricSet::new(),
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
        self.enabled_metrics = enabled;
        if found.has_errors() {
            tracing::error!("{}", pretty_print_validation_errors(&found));
            return Err(CollectorFactoryError::Configuration { results: found });
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
        let invalid = ExcludeValidation {
            invalid_metrics: metrics
                .iter()
                .filter(|metric| !DEFINED_PROMETHEUS_METRICS.contains(*metric))
                .map(|metric| (*metric).to_owned())
                .collect(),
            invalid_labels: labels
                .iter()
                .filter(|label| !defined.contains(**label))
                .map(|label| (*label).to_owned())
                .collect(),
        };
        if invalid.has_errors() {
            return Err(CollectorFactoryError::Exclusions { invalid });
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
            let kept = labelnames
                .iter()
                .copied()
                .filter(|label| !excluded_labels.contains(*label))
                .collect::<Vec<_>>();
            if kept.len() == labelnames.len() {
                return MetricLike::Real(construct(name, labelnames));
            }
            MetricLike::Excluded(ExcludedLabelMetric {
                metric: construct(name, &kept),
                excluded_labels: excluded_labels.clone(),
            })
        }
    }
}

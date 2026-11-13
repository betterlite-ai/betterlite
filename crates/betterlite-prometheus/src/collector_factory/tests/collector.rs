use std::collections::BTreeSet;

use rust_decimal::Decimal;

use crate::no_op_metric::NoOpMetric;

// decimal keeps a money increment exact; inf and NaN stay explicit instead of becoming a float
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MetricValue {
    Finite(Decimal),
    PositiveInfinity,
    NegativeInfinity,
    NotANumber,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MetricUpdate {
    Inc(MetricValue),
    Set(MetricValue),
    Observe(MetricValue),
    Remove,
}

pub trait Collector: Sized + Clone {
    #[must_use]
    fn labels(&self, labels: &[(&str, &str)]) -> Self;
    fn update(&self, update: MetricUpdate);
    fn inc(&self, value: MetricValue) {
        self.update(MetricUpdate::Inc(value));
    }
    fn set(&self, value: MetricValue) {
        self.update(MetricUpdate::Set(value));
    }
    fn observe(&self, value: MetricValue) {
        self.update(MetricUpdate::Observe(value));
    }
    fn remove(&self) {
        self.update(MetricUpdate::Remove);
    }
}

impl Collector for NoOpMetric {
    fn labels(&self, _labels: &[(&str, &str)]) -> Self {
        *self
    }
    fn update(&self, _update: MetricUpdate) {}
}

// a metric registered without the globally excluded labels: every labels(...) call drops them too
#[derive(Clone, Debug)]
pub struct ExcludedLabelMetric<M> {
    pub metric: M,
    pub excluded_labels: BTreeSet<String>,
}

impl<M: Collector> Collector for ExcludedLabelMetric<M> {
    fn labels(&self, labels: &[(&str, &str)]) -> Self {
        let kept = labels
            .iter()
            .copied()
            .filter(|(name, _)| !self.excluded_labels.contains(*name))
            .collect::<Vec<_>>();
        if kept.is_empty() {
            return self.clone();
        }
        Self {
            metric: self.metric.labels(&kept),
            excluded_labels: self.excluded_labels.clone(),
        }
    }
    fn update(&self, update: MetricUpdate) {
        self.metric.update(update);
    }
}

// LiteLLM's _MetricLike: a disabled metric, a label-filtered metric, or the registered one
#[derive(Clone, Debug)]
pub enum MetricLike<M> {
    NoOp(NoOpMetric),
    Excluded(ExcludedLabelMetric<M>),
    Real(M),
}

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

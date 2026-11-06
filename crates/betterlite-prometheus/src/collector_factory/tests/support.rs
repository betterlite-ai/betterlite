#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_prometheus::collector_factory::{Collector, LabelSchema, MetricUpdate};

#[derive(Clone, Debug)]
pub(crate) struct FakeSchema {
    pub(crate) labels: BTreeMap<String, Vec<String>>,
    pub(crate) mode: String,
}

impl FakeSchema {
    pub(crate) fn new(entries: &[(&str, &[&str])]) -> Self {
        let labels = entries
            .iter()
            .map(|(metric, names)| {
                let values = names.iter().map(|name| (*name).to_owned()).collect();
                ((*metric).to_owned(), values)
            })
            .collect();
        Self {
            labels,
            mode: String::from("api_key_alias"),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum FakeSchemaError {
    #[error("the fake schema has no labels for {metric}")]
    Missing { metric: String },
}

impl LabelSchema for FakeSchema {
    type Error = FakeSchemaError;

    fn labels(&self, metric: &str) -> Result<Vec<String>, Self::Error> {
        let labels = self.labels.get(metric).cloned();
        labels.ok_or_else(|| FakeSchemaError::Missing {
            metric: metric.to_owned(),
        })
    }

    fn caller_identity_mode(&self) -> &str {
        &self.mode
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ChangingSchema {
    pub(crate) responses: Arc<Mutex<VecDeque<Vec<String>>>>,
    pub(crate) calls: Arc<AtomicUsize>,
}

impl ChangingSchema {
    pub(crate) fn new(entries: &[&[&str]]) -> Self {
        let responses = entries
            .iter()
            .map(|names| names.iter().map(|name| (*name).to_owned()).collect())
            .collect();
        Self {
            responses: Arc::new(Mutex::new(responses)),
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl LabelSchema for ChangingSchema {
    type Error = FakeSchemaError;

    fn labels(&self, metric: &str) -> Result<Vec<String>, Self::Error> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let response = self
            .responses
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        response.ok_or_else(|| FakeSchemaError::Missing {
            metric: metric.to_owned(),
        })
    }

    fn caller_identity_mode(&self) -> &'static str {
        "api_key_alias"
    }
}

#[derive(Clone, Debug)]
pub(crate) struct FakeMetric {
    pub(crate) name: String,
    pub(crate) labelnames: Vec<String>,
    pub(crate) labels: Vec<(String, String)>,
    pub(crate) log: Arc<Mutex<Vec<String>>>,
    pub(crate) label_calls: Arc<AtomicUsize>,
}

impl FakeMetric {
    pub(crate) fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            labelnames: Vec::new(),
            labels: Vec::new(),
            log: Arc::new(Mutex::new(Vec::new())),
            label_calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub(crate) fn registered(name: &str, labelnames: &[&str]) -> Self {
        let mut metric = Self::new(name);
        metric.labelnames = labelnames.iter().map(|label| (*label).to_owned()).collect();
        metric
    }

    pub(crate) fn entries(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }

    pub(crate) fn label_calls(&self) -> usize {
        self.label_calls.load(Ordering::SeqCst)
    }
}

impl Collector for FakeMetric {
    fn labels(&self, labels: &[(&str, &str)]) -> Self {
        self.label_calls.fetch_add(1, Ordering::SeqCst);
        let mut child = self.clone();
        child.labels.extend(
            labels
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned())),
        );
        child
    }

    fn update(&self, update: MetricUpdate) {
        self.log
            .lock()
            .unwrap()
            .push(format!("{}:{update:?}", self.name));
    }
}

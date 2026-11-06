use std::collections::BTreeSet;
use std::time::Duration;

use opentelemetry::Context;
use opentelemetry::KeyValue;
use opentelemetry::baggage::BaggageExt as _;
use opentelemetry::trace::Span as _;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::trace::{
    BatchSpanProcessor, SimpleSpanProcessor, Span, SpanData, SpanProcessor,
};

use crate::otel::plumbing::sinks::SpanSink;
use crate::otel::vocabulary::constants::attribute_keys::litellm::METADATA_PREFIX;

// Stamps an allowlisted set of baggage entries onto every span at start.
#[derive(Debug)]
pub struct BaggageSpanProcessor {
    allowed_keys: BTreeSet<String>,
    allowed_prefixes: Vec<String>,
}

impl BaggageSpanProcessor {
    #[must_use]
    pub fn new(allowed_keys: Vec<String>, allowed_prefixes: Option<Vec<String>>) -> Self {
        Self {
            allowed_keys: allowed_keys.into_iter().collect(),
            allowed_prefixes: allowed_prefixes.unwrap_or_else(|| vec![METADATA_PREFIX.to_owned()]),
        }
    }

    #[must_use]
    pub fn is_allowed(&self, key: &str) -> bool {
        self.allowed_keys.contains(key)
            || self
                .allowed_prefixes
                .iter()
                .any(|prefix| key.starts_with(prefix.as_str()))
    }
}

impl SpanProcessor for BaggageSpanProcessor {
    fn on_start(&self, span: &mut Span, cx: &Context) {
        for (key, (value, _metadata)) in cx.baggage() {
            if self.is_allowed(key.as_str()) {
                span.set_attribute(KeyValue::new(key.clone(), value.clone()));
            }
        }
    }

    fn on_end(&self, _span: SpanData) {}

    fn force_flush(&self) -> OTelSdkResult {
        Ok(())
    }

    fn shutdown_with_timeout(&self, _timeout: Duration) -> OTelSdkResult {
        Ok(())
    }
}

#[derive(Debug)]
pub enum ExportProcessor {
    Simple(Box<SimpleSpanProcessor<SpanSink>>),
    Batch(BatchSpanProcessor),
}

// Console and in-memory spans export synchronously, which tests rely on; everything else batches.
#[must_use]
pub fn processor_for(exporter: SpanSink, use_simple: Option<bool>) -> ExportProcessor {
    let synchronous = matches!(exporter, SpanSink::Console(_) | SpanSink::InMemory(_));
    let simple = use_simple.unwrap_or(synchronous);
    if simple {
        ExportProcessor::Simple(Box::new(SimpleSpanProcessor::new(exporter)))
    } else {
        ExportProcessor::Batch(BatchSpanProcessor::builder(exporter).build())
    }
}

impl SpanProcessor for ExportProcessor {
    fn on_start(&self, span: &mut Span, cx: &Context) {
        match self {
            Self::Simple(processor) => processor.on_start(span, cx),
            Self::Batch(processor) => processor.on_start(span, cx),
        }
    }

    fn on_end(&self, span: SpanData) {
        match self {
            Self::Simple(processor) => processor.on_end(span),
            Self::Batch(processor) => processor.on_end(span),
        }
    }

    fn force_flush(&self) -> OTelSdkResult {
        match self {
            Self::Simple(processor) => processor.force_flush(),
            Self::Batch(processor) => processor.force_flush(),
        }
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        match self {
            Self::Simple(processor) => processor.shutdown_with_timeout(timeout),
            Self::Batch(processor) => processor.shutdown_with_timeout(timeout),
        }
    }

    fn set_resource(&mut self, resource: &Resource) {
        match self {
            Self::Simple(processor) => processor.set_resource(resource),
            Self::Batch(processor) => processor.set_resource(resource),
        }
    }
}

#[path = "processors/tests.rs"]
mod tests;

use std::time::Duration;

use opentelemetry_sdk::Resource;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::logs::{InMemoryLogExporter, LogBatch, LogExporter};
use opentelemetry_sdk::metrics::Temporality;
use opentelemetry_sdk::metrics::data::ResourceMetrics;
use opentelemetry_sdk::metrics::exporter::PushMetricExporter;
use opentelemetry_sdk::trace::{InMemorySpanExporter, SpanData, SpanExporter};

// The SDK exporter traits are not object safe, so each signal picks among its exporters with an enum.
#[derive(Debug)]
pub enum SpanSink {
    Console(opentelemetry_stdout::SpanExporter),
    InMemory(InMemorySpanExporter),
    Otlp(Box<opentelemetry_otlp::SpanExporter>),
}

impl SpanExporter for SpanSink {
    async fn export(&self, batch: Vec<SpanData>) -> OTelSdkResult {
        match self {
            Self::Console(exporter) => exporter.export(batch).await,
            Self::InMemory(exporter) => exporter.export(batch).await,
            Self::Otlp(exporter) => exporter.export(batch).await,
        }
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        match self {
            Self::Console(exporter) => exporter.shutdown_with_timeout(timeout),
            Self::InMemory(exporter) => exporter.shutdown_with_timeout(timeout),
            Self::Otlp(exporter) => exporter.shutdown_with_timeout(timeout),
        }
    }

    fn force_flush(&self) -> OTelSdkResult {
        match self {
            Self::Console(exporter) => exporter.force_flush(),
            Self::InMemory(exporter) => exporter.force_flush(),
            Self::Otlp(exporter) => exporter.force_flush(),
        }
    }

    fn set_resource(&mut self, resource: &Resource) {
        match self {
            Self::Console(exporter) => exporter.set_resource(resource),
            Self::InMemory(exporter) => exporter.set_resource(resource),
            Self::Otlp(exporter) => exporter.set_resource(resource),
        }
    }
}

#[derive(Debug)]
pub enum LogSink {
    Console(opentelemetry_stdout::LogExporter),
    InMemory(InMemoryLogExporter),
    Otlp(Box<opentelemetry_otlp::LogExporter>),
}

impl LogExporter for LogSink {
    async fn export(&self, batch: LogBatch<'_>) -> OTelSdkResult {
        match self {
            Self::Console(exporter) => exporter.export(batch).await,
            Self::InMemory(exporter) => exporter.export(batch).await,
            Self::Otlp(exporter) => exporter.export(batch).await,
        }
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        match self {
            Self::Console(exporter) => exporter.shutdown_with_timeout(timeout),
            Self::InMemory(exporter) => exporter.shutdown_with_timeout(timeout),
            Self::Otlp(exporter) => exporter.shutdown_with_timeout(timeout),
        }
    }

    fn set_resource(&mut self, resource: &Resource) {
        match self {
            Self::Console(exporter) => exporter.set_resource(resource),
            Self::InMemory(exporter) => exporter.set_resource(resource),
            Self::Otlp(exporter) => exporter.set_resource(resource),
        }
    }
}

#[derive(Debug)]
pub enum MetricSink {
    Console(Box<opentelemetry_stdout::MetricExporter>),
    Otlp(Box<opentelemetry_otlp::MetricExporter>),
}

impl PushMetricExporter for MetricSink {
    async fn export(&self, metrics: &ResourceMetrics) -> OTelSdkResult {
        match self {
            Self::Console(exporter) => exporter.export(metrics).await,
            Self::Otlp(exporter) => exporter.export(metrics).await,
        }
    }

    fn force_flush(&self) -> OTelSdkResult {
        match self {
            Self::Console(exporter) => exporter.force_flush(),
            Self::Otlp(exporter) => exporter.force_flush(),
        }
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        match self {
            Self::Console(exporter) => exporter.shutdown_with_timeout(timeout),
            Self::Otlp(exporter) => exporter.shutdown_with_timeout(timeout),
        }
    }

    fn temporality(&self) -> Temporality {
        match self {
            Self::Console(exporter) => exporter.temporality(),
            Self::Otlp(exporter) => exporter.temporality(),
        }
    }
}

#[path = "sinks/tests.rs"]
mod tests;

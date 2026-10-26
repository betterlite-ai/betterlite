#![cfg(test)]

use std::io;
use std::sync::{Arc, Mutex};

use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};
use tracing::subscriber::with_default;

use super::ActiveRecorderRegistry;
use crate::billable_category::BillableCategory;
use crate::recorder::BillingMetricsRecorder;

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn logged<F>(action: F) -> String
where
    F: FnOnce(),
{
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    with_default(subscriber, action);
    String::from_utf8(captured.0.lock().unwrap().clone()).unwrap()
}

fn in_memory() -> (BillingMetricsRecorder, InMemoryMetricExporter) {
    let exporter = InMemoryMetricExporter::default();
    let reader = PeriodicReader::builder(exporter.clone()).build();
    let provider = SdkMeterProvider::builder().with_reader(reader).build();
    (BillingMetricsRecorder::new(provider), exporter)
}

#[test]
fn an_empty_registry_pops_nothing() {
    let registry = ActiveRecorderRegistry::default();
    assert!(registry.pop().is_none());
}

#[test]
fn pop_hands_back_the_recorder_once() {
    let registry = ActiveRecorderRegistry::default();
    let (recorder, exporter) = in_memory();
    let output = logged(|| registry.set(recorder));
    assert_eq!(output, "");
    let popped = registry.pop().unwrap();
    assert!(registry.pop().is_none());
    popped.record(BillableCategory::Llm, "/v1/completions", 200, None);
    popped.shutdown();
    assert_eq!(exporter.get_finished_metrics().unwrap().len(), 1);
}

#[test]
fn set_warns_when_it_replaces_an_active_recorder() {
    let registry = ActiveRecorderRegistry::default();
    let (first, first_exporter) = in_memory();
    let (second, second_exporter) = in_memory();
    registry.set(first);
    let output = logged(|| registry.set(second));
    assert!(output.contains("WARN"), "{output}");
    assert!(output.contains("replaced the active one"), "{output}");
    let active = registry.pop().unwrap();
    active.record(BillableCategory::Mcp, "/mcp", 200, None);
    active.shutdown();
    assert!(registry.pop().is_none());
    assert!(first_exporter.get_finished_metrics().unwrap().is_empty());
    assert_eq!(second_exporter.get_finished_metrics().unwrap().len(), 1);
}

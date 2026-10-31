#![cfg(test)]

use super::ActiveRecorderRegistry;
use crate::billable_category::BillableCategory;
use crate::tests::{in_memory, logged};

#[test]
fn pop_hands_back_the_recorder_once() {
    let registry = ActiveRecorderRegistry::default();
    assert!(registry.pop().is_none());
    let (recorder, exporter) = in_memory();
    let ((), output) = logged(|| registry.set(recorder));
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
    let ((), output) = logged(|| registry.set(second));
    assert!(output.contains("WARN"), "{output}");
    assert!(output.contains("replaced the active one"), "{output}");
    let active = registry.pop().unwrap();
    active.record(BillableCategory::Mcp, "/mcp", 200, None);
    active.shutdown();
    assert!(first_exporter.get_finished_metrics().unwrap().is_empty());
    assert_eq!(second_exporter.get_finished_metrics().unwrap().len(), 1);
}

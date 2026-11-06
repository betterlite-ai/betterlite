#![cfg(test)]

use betterlite_prometheus::collector_factory::registry::{
    CallbackEntry, CallbackRegistry, PROMETHEUS_LOGGER_CLASS,
};

#[derive(Debug, Eq, PartialEq)]
struct FakeLogger {
    name: &'static str,
}

static LOGGER: FakeLogger = FakeLogger { name: "logger" };
static OTHER_LOGGER: FakeLogger = FakeLogger {
    name: "other-prometheus-logger",
};

fn entry(
    class_name: &str,
    logger: Option<&'static FakeLogger>,
) -> CallbackEntry<'static, FakeLogger> {
    CallbackEntry {
        class_name: class_name.to_owned(),
        logger,
    }
}

#[test]
fn prometheus_registry_returns_the_first_logger() {
    let registry = CallbackRegistry {
        entries: vec![
            entry(PROMETHEUS_LOGGER_CLASS, Some(&LOGGER)),
            entry(PROMETHEUS_LOGGER_CLASS, Some(&OTHER_LOGGER)),
        ],
    };
    assert_eq!(registry.get_instance(), Some(&LOGGER));
}

#[test]
fn prometheus_registry_skips_other_callbacks() {
    let registry = CallbackRegistry {
        entries: vec![
            entry("CustomLogger", None),
            entry(PROMETHEUS_LOGGER_CLASS, Some(&LOGGER)),
        ],
    };
    assert_eq!(registry.get_instance(), Some(&LOGGER));
}

#[test]
fn prometheus_registry_returns_none_when_not_registered() {
    let registry = CallbackRegistry {
        entries: vec![entry("CustomLogger", None)],
    };
    assert_eq!(registry.get_instance(), None);
    let empty: CallbackRegistry<'static, FakeLogger> = CallbackRegistry { entries: vec![] };
    assert_eq!(empty.get_instance(), None);
}

#[test]
fn prometheus_registry_reads_the_callbacks_at_call_time() {
    let mut registry = CallbackRegistry { entries: vec![] };
    assert_eq!(registry.get_instance(), None);
    registry
        .entries
        .push(entry(PROMETHEUS_LOGGER_CLASS, Some(&LOGGER)));
    assert_eq!(registry.get_instance(), Some(&LOGGER));
}

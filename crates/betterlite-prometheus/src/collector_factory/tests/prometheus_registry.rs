#![cfg(test)]

use betterlite_prometheus::collector_factory::registry::CallbackSlots;

#[derive(Debug, Eq, PartialEq)]
struct FakeLogger {
    name: &'static str,
}

static FIRST: FakeLogger = FakeLogger { name: "first" };
static SECOND: FakeLogger = FakeLogger { name: "second" };
static LOGGER: FakeLogger = FakeLogger { name: "logger" };

struct FakeCallbacks {
    slots: Vec<Option<&'static FakeLogger>>,
}

impl CallbackSlots for FakeCallbacks {
    type Logger = FakeLogger;

    fn registered_callbacks(&self) -> &[Option<&FakeLogger>] {
        &self.slots
    }
}

#[test]
fn prometheus_registry_returns_the_first_logger() {
    let callbacks = FakeCallbacks {
        slots: vec![Some(&FIRST), Some(&SECOND)],
    };
    assert_eq!(callbacks.get_instance(), Some(&FIRST));
}

#[test]
fn prometheus_registry_skips_other_callbacks() {
    let callbacks = FakeCallbacks {
        slots: vec![None, Some(&LOGGER)],
    };
    assert_eq!(callbacks.get_instance(), Some(&LOGGER));
}

#[test]
fn prometheus_registry_returns_none_when_not_registered() {
    let callbacks = FakeCallbacks { slots: vec![None] };
    assert_eq!(callbacks.get_instance(), None);
}

#[test]
fn prometheus_registry_reads_the_slots_at_call_time() {
    let mut callbacks = FakeCallbacks { slots: vec![None] };
    assert_eq!(callbacks.get_instance(), None);
    callbacks.slots.push(Some(&LOGGER));
    assert_eq!(callbacks.get_instance(), Some(&LOGGER));
}

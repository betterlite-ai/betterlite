#![cfg(test)]

use betterlite_prometheus::collector_factory::get_instance;

#[derive(Debug, Eq, PartialEq)]
struct FakeLogger {
    name: &'static str,
}

#[test]
fn prometheus_registry_returns_the_first_logger() {
    let first = FakeLogger { name: "first" };
    let second = FakeLogger { name: "second" };
    let callbacks = [Some(&first), Some(&second)];
    assert_eq!(get_instance(&callbacks), Some(&first));
}

#[test]
fn prometheus_registry_skips_other_callbacks() {
    let logger = FakeLogger { name: "logger" };
    let callbacks = [None, Some(&logger)];
    assert_eq!(get_instance(&callbacks), Some(&logger));
}

#[test]
fn prometheus_registry_returns_none_when_not_registered() {
    let callbacks: [Option<&FakeLogger>; 1] = [None];
    assert_eq!(get_instance(&callbacks), None);
}

#[test]
fn prometheus_registry_reads_the_slots_at_call_time() {
    let logger = FakeLogger { name: "logger" };
    let mut callbacks: Vec<Option<&FakeLogger>> = vec![None];
    assert_eq!(get_instance(&callbacks), None);
    callbacks.push(Some(&logger));
    assert_eq!(get_instance(&callbacks), Some(&logger));
}

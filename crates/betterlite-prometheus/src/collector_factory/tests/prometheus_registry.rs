#![cfg(test)]

#[derive(Debug, Eq, PartialEq)]
struct FakeLogger {
    name: &'static str,
}

static FIRST: FakeLogger = FakeLogger { name: "first" };
static SECOND: FakeLogger = FakeLogger { name: "second" };
static LOGGER: FakeLogger = FakeLogger { name: "logger" };

#[test]
fn prometheus_registry_returns_the_first_logger() {
    let callbacks = [Some(&FIRST), Some(&SECOND)];
    assert_eq!(callbacks.iter().find_map(|slot| *slot), Some(&FIRST));
}

#[test]
fn prometheus_registry_skips_other_callbacks() {
    let callbacks = [None, Some(&LOGGER)];
    assert_eq!(callbacks.iter().find_map(|slot| *slot), Some(&LOGGER));
}

#[test]
fn prometheus_registry_returns_none_when_not_registered() {
    let callbacks: [Option<&FakeLogger>; 1] = [None];
    assert_eq!(callbacks.iter().find_map(|slot| *slot), None);
}

#[test]
fn prometheus_registry_reads_the_slots_at_call_time() {
    let mut callbacks: Vec<Option<&FakeLogger>> = vec![None];
    assert_eq!(callbacks.iter().find_map(|slot| *slot), None);
    callbacks.push(Some(&LOGGER));
    assert_eq!(callbacks.iter().find_map(|slot| *slot), Some(&LOGGER));
}

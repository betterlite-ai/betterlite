#![cfg(test)]

use std::sync::Arc;

use tracing::Level;

use crate::finops::focus::export_schedule::ScheduleTrigger;
use crate::finops::vantage::export::VantageExportLogger;
use crate::finops::vantage::store::{VantageCallbackList, VantageLoggers};
use crate::finops::vantage::support::{self, FakeVantageLogger};

#[test]
fn one_instance_registered_in_two_lists_appears_once() {
    let store = VantageLoggers::new();
    let logger: Arc<dyn VantageExportLogger> = Arc::new(FakeVantageLogger {
        trigger: ScheduleTrigger::Interval { seconds: 30 },
    });
    store.register(Arc::clone(&logger), VantageCallbackList::Success);
    store.register(Arc::clone(&logger), VantageCallbackList::Callbacks);
    let snapshot = store.snapshot();
    assert_eq!(snapshot.len(), 1);
    assert!(Arc::ptr_eq(&snapshot[0], &logger));
}

#[test]
fn registering_the_same_instance_twice_in_one_list_keeps_its_first_place() {
    let store = VantageLoggers::new();
    let first: Arc<dyn VantageExportLogger> = Arc::new(FakeVantageLogger {
        trigger: ScheduleTrigger::Interval { seconds: 10 },
    });
    let second: Arc<dyn VantageExportLogger> = Arc::new(FakeVantageLogger {
        trigger: ScheduleTrigger::Interval { seconds: 20 },
    });
    store.register(Arc::clone(&first), VantageCallbackList::Callbacks);
    store.register(Arc::clone(&second), VantageCallbackList::Callbacks);
    store.register(Arc::clone(&first), VantageCallbackList::Callbacks);
    let snapshot = store.snapshot();
    assert_eq!(snapshot.len(), 2);
    assert!(Arc::ptr_eq(&snapshot[0], &first));
    assert!(Arc::ptr_eq(&snapshot[1], &second));
}

#[test]
fn a_second_instance_with_the_same_registration_key_is_not_added() {
    let (logs, _guard) = support::capture_logs();
    let store = VantageLoggers::new();
    let first: Arc<dyn VantageExportLogger> = Arc::new(FakeVantageLogger {
        trigger: ScheduleTrigger::Interval { seconds: 30 },
    });
    let duplicate: Arc<dyn VantageExportLogger> = Arc::new(FakeVantageLogger {
        trigger: ScheduleTrigger::Interval { seconds: 30 },
    });
    store.register(Arc::clone(&first), VantageCallbackList::Callbacks);
    assert!(!logs.has_at(
        Level::DEBUG,
        "callback already registered in this list; keeping the first"
    ));
    store.register(Arc::clone(&duplicate), VantageCallbackList::Callbacks);
    assert!(logs.has_at(
        Level::DEBUG,
        "callback already registered in this list; keeping the first"
    ));
    let snapshot = store.snapshot();
    assert_eq!(snapshot.len(), 1);
    assert!(Arc::ptr_eq(&snapshot[0], &first));
}

#[test]
fn the_same_registration_key_in_two_lists_stays_selectable_twice() {
    let store = VantageLoggers::new();
    let first: Arc<dyn VantageExportLogger> = Arc::new(FakeVantageLogger {
        trigger: ScheduleTrigger::Interval { seconds: 30 },
    });
    let second: Arc<dyn VantageExportLogger> = Arc::new(FakeVantageLogger {
        trigger: ScheduleTrigger::Interval { seconds: 30 },
    });
    store.register(Arc::clone(&first), VantageCallbackList::Callbacks);
    store.register(Arc::clone(&second), VantageCallbackList::Success);
    let snapshot = store.snapshot();
    assert_eq!(snapshot.len(), 2);
    assert!(Arc::ptr_eq(&snapshot[0], &first));
    assert!(Arc::ptr_eq(&snapshot[1], &second));
}

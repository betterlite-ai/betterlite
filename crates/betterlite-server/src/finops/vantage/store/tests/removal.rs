#![cfg(test)]

use std::sync::Arc;

use crate::finops::focus::export_schedule::ScheduleTrigger;
use crate::finops::vantage::export::VantageExportLogger;
use crate::finops::vantage::store::{VantageCallbackList, VantageLoggers};
use crate::finops::vantage::support::FakeVantageLogger;

#[test]
fn removing_the_general_list_keeps_the_other_registrations() {
    let store = VantageLoggers::new();
    let general_only: Arc<dyn VantageExportLogger> = Arc::new(FakeVantageLogger {
        trigger: ScheduleTrigger::Interval { seconds: 10 },
    });
    let shared: Arc<dyn VantageExportLogger> = Arc::new(FakeVantageLogger {
        trigger: ScheduleTrigger::Interval { seconds: 20 },
    });
    let failure_only: Arc<dyn VantageExportLogger> = Arc::new(FakeVantageLogger {
        trigger: ScheduleTrigger::Interval { seconds: 30 },
    });
    store.register(Arc::clone(&general_only), VantageCallbackList::Callbacks);
    store.register(Arc::clone(&shared), VantageCallbackList::Callbacks);
    store.register(Arc::clone(&shared), VantageCallbackList::Success);
    store.register(Arc::clone(&failure_only), VantageCallbackList::Failure);
    assert_eq!(store.remove_from_list(VantageCallbackList::Callbacks), 2);
    let snapshot = store.snapshot();
    assert_eq!(snapshot.len(), 2);
    assert!(Arc::ptr_eq(&snapshot[0], &shared));
    assert!(Arc::ptr_eq(&snapshot[1], &failure_only));
}

#[test]
fn removing_an_empty_list_removes_nothing() {
    let store = VantageLoggers::new();
    let logger: Arc<dyn VantageExportLogger> = Arc::new(FakeVantageLogger {
        trigger: ScheduleTrigger::Interval { seconds: 30 },
    });
    store.register(Arc::clone(&logger), VantageCallbackList::Success);
    assert_eq!(store.remove_from_list(VantageCallbackList::Callbacks), 0);
    let snapshot = store.snapshot();
    assert_eq!(snapshot.len(), 1);
    assert!(Arc::ptr_eq(&snapshot[0], &logger));
}

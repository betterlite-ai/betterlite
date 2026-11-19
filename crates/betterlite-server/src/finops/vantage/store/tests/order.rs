#![cfg(test)]

use std::sync::Arc;

use crate::finops::focus::export_schedule::ScheduleTrigger;
use crate::finops::vantage::export::VantageExportLogger;
use crate::finops::vantage::store::{VantageCallbackList, VantageLoggers};
use crate::finops::vantage::support::FakeVantageLogger;

#[test]
fn the_snapshot_follows_the_callback_list_order() {
    let store = VantageLoggers::new();
    let success: Arc<dyn VantageExportLogger> = Arc::new(FakeVantageLogger {
        trigger: ScheduleTrigger::Interval { seconds: 30 },
    });
    let general: Arc<dyn VantageExportLogger> = Arc::new(FakeVantageLogger {
        trigger: ScheduleTrigger::Interval { seconds: 60 },
    });
    let failure: Arc<dyn VantageExportLogger> = Arc::new(FakeVantageLogger {
        trigger: ScheduleTrigger::Interval { seconds: 90 },
    });
    store.register(Arc::clone(&success), VantageCallbackList::Success);
    store.register(Arc::clone(&general), VantageCallbackList::Callbacks);
    store.register(Arc::clone(&failure), VantageCallbackList::Failure);
    let snapshot = store.snapshot();
    assert_eq!(snapshot.len(), 3);
    assert!(Arc::ptr_eq(&snapshot[0], &general));
    assert!(Arc::ptr_eq(&snapshot[1], &success));
    assert!(Arc::ptr_eq(&snapshot[2], &failure));
}

#[test]
fn the_async_lists_come_after_the_synchronous_ones() {
    let store = VantageLoggers::new();
    let async_failure: Arc<dyn VantageExportLogger> = Arc::new(FakeVantageLogger {
        trigger: ScheduleTrigger::Interval { seconds: 10 },
    });
    let async_success: Arc<dyn VantageExportLogger> = Arc::new(FakeVantageLogger {
        trigger: ScheduleTrigger::Interval { seconds: 20 },
    });
    let general: Arc<dyn VantageExportLogger> = Arc::new(FakeVantageLogger {
        trigger: ScheduleTrigger::Interval { seconds: 30 },
    });
    store.register(
        Arc::clone(&async_failure),
        VantageCallbackList::AsyncFailure,
    );
    store.register(
        Arc::clone(&async_success),
        VantageCallbackList::AsyncSuccess,
    );
    store.register(Arc::clone(&general), VantageCallbackList::Callbacks);
    let snapshot = store.snapshot();
    assert_eq!(snapshot.len(), 3);
    assert!(Arc::ptr_eq(&snapshot[0], &general));
    assert!(Arc::ptr_eq(&snapshot[1], &async_success));
    assert!(Arc::ptr_eq(&snapshot[2], &async_failure));
}

#[test]
fn every_list_holds_its_place_in_the_snapshot() {
    let store = VantageLoggers::new();
    let callbacks: Arc<dyn VantageExportLogger> = Arc::new(FakeVantageLogger {
        trigger: ScheduleTrigger::Interval { seconds: 10 },
    });
    let success: Arc<dyn VantageExportLogger> = Arc::new(FakeVantageLogger {
        trigger: ScheduleTrigger::Interval { seconds: 20 },
    });
    let failure: Arc<dyn VantageExportLogger> = Arc::new(FakeVantageLogger {
        trigger: ScheduleTrigger::Interval { seconds: 30 },
    });
    let async_success: Arc<dyn VantageExportLogger> = Arc::new(FakeVantageLogger {
        trigger: ScheduleTrigger::Interval { seconds: 40 },
    });
    let async_failure: Arc<dyn VantageExportLogger> = Arc::new(FakeVantageLogger {
        trigger: ScheduleTrigger::Interval { seconds: 50 },
    });
    store.register(
        Arc::clone(&async_failure),
        VantageCallbackList::AsyncFailure,
    );
    store.register(
        Arc::clone(&async_success),
        VantageCallbackList::AsyncSuccess,
    );
    store.register(Arc::clone(&failure), VantageCallbackList::Failure);
    store.register(Arc::clone(&success), VantageCallbackList::Success);
    store.register(Arc::clone(&callbacks), VantageCallbackList::Callbacks);
    let snapshot = store.snapshot();
    assert_eq!(snapshot.len(), 5);
    assert!(Arc::ptr_eq(&snapshot[0], &callbacks));
    assert!(Arc::ptr_eq(&snapshot[1], &success));
    assert!(Arc::ptr_eq(&snapshot[2], &failure));
    assert!(Arc::ptr_eq(&snapshot[3], &async_success));
    assert!(Arc::ptr_eq(&snapshot[4], &async_failure));
}

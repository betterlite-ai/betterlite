#![cfg(test)]

use std::sync::Arc;

use crate::finops::focus::export_schedule::ScheduleTrigger;
use crate::finops::vantage::export::VantageExportLogger;
use crate::finops::vantage::export_schedule::init_vantage_background_job;
use crate::finops::vantage::store::{VantageCallbackList, VantageLoggers};
use crate::finops::vantage::support::{FakeVantageLogger, RecordingScheduler};

#[test]
fn a_registered_implementor_is_selected_without_an_exact_struct_match() {
    let store = VantageLoggers::new();
    let first: Arc<dyn VantageExportLogger> = Arc::new(FakeVantageLogger {
        trigger: ScheduleTrigger::Interval { seconds: 30 },
    });
    store.register(Arc::clone(&first), VantageCallbackList::Callbacks);
    store.register(
        Arc::new(FakeVantageLogger {
            trigger: ScheduleTrigger::Interval { seconds: 60 },
        }),
        VantageCallbackList::Success,
    );
    let scheduler = RecordingScheduler::new(false);
    init_vantage_background_job(&store, &scheduler).expect("the job registers");
    let jobs = scheduler.jobs();
    assert_eq!(jobs.len(), 1);
    assert!(Arc::ptr_eq(&jobs[0].0, &first));
    assert_eq!(jobs[0].1, ScheduleTrigger::Interval { seconds: 30 });
}

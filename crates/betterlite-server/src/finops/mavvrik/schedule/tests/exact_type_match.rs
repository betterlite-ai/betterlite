#![cfg(test)]

use std::sync::Arc;

use crate::finops::mavvrik::schedule::{
    MavvrikLoggerType, RegisteredMavvrikLogger, init_mavvrik_background_job,
};
use crate::finops::mavvrik::support::{
    FakeLazyInit, Recorder, RecordingScheduler, build, environment,
};

#[test]
fn a_subclass_is_filtered_out_and_the_exact_logger_is_scheduled() {
    let recorder = Recorder::default();
    let subclass = Arc::new(build(&environment(&[]), &recorder, None));
    let exact = Arc::new(build(&environment(&[]), &recorder, None));
    let registered = vec![
        RegisteredMavvrikLogger {
            logger: Arc::clone(&subclass),
            logger_type: MavvrikLoggerType::Subclass,
        },
        RegisteredMavvrikLogger {
            logger: Arc::clone(&exact),
            logger_type: MavvrikLoggerType::Mavvrik,
        },
    ];
    let lazy_init = FakeLazyInit::new(Some(Arc::clone(&subclass)));
    let scheduler = RecordingScheduler::new(false);
    init_mavvrik_background_job(&registered, true, &lazy_init, &scheduler)
        .expect("the job is scheduled");
    let jobs = scheduler.jobs();
    assert_eq!(jobs.len(), 1);
    assert!(Arc::ptr_eq(&jobs[0].0, &exact));
    assert!(lazy_init.integrations().is_empty());
}

#[test]
fn the_first_exact_logger_is_the_one_scheduled() {
    let recorder = Recorder::default();
    let first = Arc::new(build(&environment(&[]), &recorder, None));
    let second = Arc::new(build(&environment(&[]), &recorder, None));
    let registered = vec![
        RegisteredMavvrikLogger {
            logger: Arc::clone(&first),
            logger_type: MavvrikLoggerType::Mavvrik,
        },
        RegisteredMavvrikLogger {
            logger: Arc::clone(&second),
            logger_type: MavvrikLoggerType::Mavvrik,
        },
    ];
    let lazy_init = FakeLazyInit::new(None);
    let scheduler = RecordingScheduler::new(false);
    init_mavvrik_background_job(&registered, true, &lazy_init, &scheduler)
        .expect("the job is scheduled");
    let jobs = scheduler.jobs();
    assert_eq!(jobs.len(), 1);
    assert!(Arc::ptr_eq(&jobs[0].0, &first));
}

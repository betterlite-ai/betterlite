#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::atomic::AtomicUsize;

use crate::finops::focus::error::FocusError;
use crate::finops::focus::support::{self, ExportCall, FlakyFactory, Recorder, at};

#[tokio::test]
async fn absent_bounds_export_everything_with_the_limit() {
    let recorder = Recorder::new(false);
    let logger = support::build(
        &BTreeMap::new(),
        support::params(),
        recorder.factory(),
        None,
    );
    logger
        .export_usage_data(Some(10), None, None)
        .await
        .expect("the fake export succeeds");
    assert_eq!(recorder.calls(), vec![ExportCall::All { limit: Some(10) }]);
}

#[tokio::test]
async fn both_bounds_export_exactly_that_window_tagged_with_the_frequency() {
    let recorder = Recorder::new(false);
    let mut params = support::params();
    params.frequency = Some("daily".to_owned());
    let logger = support::build(&BTreeMap::new(), params, recorder.factory(), None);
    let start = at("2024-03-09T00:00:00Z");
    let end = at("2024-03-10T00:00:00Z");
    logger
        .export_usage_data(None, Some(start), Some(end))
        .await
        .expect("the fake export succeeds");
    assert_eq!(
        recorder.calls(),
        vec![ExportCall::Window {
            start,
            end,
            frequency: "daily".to_owned(),
            limit: None,
        }]
    );
}

#[tokio::test]
async fn the_engine_is_built_once_across_several_exports() {
    let recorder = Recorder::new(false);
    let logger = support::build(
        &BTreeMap::new(),
        support::params(),
        recorder.factory(),
        None,
    );
    logger.export_usage_data(None, None, None).await.unwrap();
    logger.export_usage_data(Some(1), None, None).await.unwrap();
    assert_eq!(recorder.settings().len(), 1);
    assert_eq!(
        recorder.calls(),
        vec![
            ExportCall::All { limit: None },
            ExportCall::All { limit: Some(1) },
        ]
    );
}

#[tokio::test]
async fn a_failed_build_is_retried_on_the_next_export() {
    let recorder = Recorder::new(false);
    let factory = FlakyFactory {
        recorder: recorder.clone(),
        attempts: AtomicUsize::new(0),
    };
    let logger = support::build(&BTreeMap::new(), support::params(), factory, None);
    let first = logger.export_usage_data(None, None, None).await;
    let Err(error) = first else {
        panic!("the first build fails");
    };
    assert!(matches!(&error, FocusError::Export(_)));
    logger
        .export_usage_data(None, None, None)
        .await
        .expect("the retried build succeeds");
    assert_eq!(recorder.settings().len(), 1);
    assert_eq!(recorder.calls(), vec![ExportCall::All { limit: None }]);
}

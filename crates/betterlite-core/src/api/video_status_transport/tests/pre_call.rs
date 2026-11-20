#![cfg(test)]

use betterlite_models::decisions::Json;

use super::harness::{Call, harness};
use super::support::{
    API_BASE, COMPLETE_URL, Kind, base_params, conversions, json, operation, plain_headers,
    pre_calls, sync_sends,
};
use crate::api::video_status_transport::error::VideoStatusError;
use crate::api::video_status_transport::records::Params;

#[test]
fn pre_call_records_the_documented_payload() {
    let mut run = harness();
    run.deps.config.transformed = Ok((COMPLETE_URL.to_owned(), operation()));
    let params = base_params(API_BASE);
    let headers = plain_headers();
    let mut call = Call::new(&params);
    call.extra_headers = Some(&headers);
    run.sync(call).expect("the status call succeeds");
    let records = pre_calls(&run.events());
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].input, Json::Text(String::new()));
    assert_eq!(
        records[0].additional_args,
        json(
            r#"{"api_base":"https://built/videos/video_1","headers":{"x-caller":"given"},"video_id":"video_1","data":{"operationName":"projects/p/operations/o"}}"#
        )
    );
}

#[test]
fn pre_call_keeps_the_data_key_for_an_empty_map() {
    let mut run = harness();
    let params = Params::new();
    run.sync(Call::new(&params))
        .expect("the status call succeeds");
    let records = pre_calls(&run.events());
    assert_eq!(
        records[0].additional_args,
        json(
            r#"{"api_base":"https://built/videos/video_1","headers":{},"video_id":"video_1","data":{}}"#
        )
    );
}

#[test]
fn pre_call_runs_once_before_the_send() {
    let mut run = harness();
    let params = Params::new();
    run.sync(Call::new(&params))
        .expect("the status call succeeds");
    let kinds = run.events().kinds;
    assert_eq!(
        kinds.iter().filter(|kind| **kind == Kind::PreCall).count(),
        1
    );
    let recorded = kinds
        .iter()
        .position(|kind| *kind == Kind::PreCall)
        .expect("pre_call ran");
    let sent = kinds
        .iter()
        .position(|kind| *kind == Kind::SyncSend)
        .expect("the send ran");
    assert!(recorded < sent);
}

#[test]
fn pre_call_failure_stops_the_call_before_the_send() {
    let mut run = harness();
    run.state.request_logging.raw_request_only = true;
    let params = Params::new();
    let error = run
        .sync(Call::new(&params))
        .expect_err("the raw-request-only flag stops the call");
    let VideoStatusError::Logging(_) = error else {
        panic!("pre_call's own error, not a converted one")
    };
    let events = run.events();
    assert_eq!(pre_calls(&events).len(), 1);
    assert!(sync_sends(&events).is_empty());
    assert!(conversions(&events).is_empty());
}

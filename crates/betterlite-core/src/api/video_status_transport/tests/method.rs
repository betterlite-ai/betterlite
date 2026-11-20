#![cfg(test)]

use super::harness::{Call, harness};
use super::support::{COMPLETE_URL, async_sends, json, operation, sync_sends};
use crate::api::video_status_transport::records::{Params, StatusMethod};

#[test]
fn method_choice_empty_data_sends_a_get() {
    let mut run = harness();
    let params = Params::new();
    run.sync(Call::new(&params))
        .expect("the status call succeeds");
    let sends = sync_sends(&run.events());
    assert_eq!(sends[0].1.method, StatusMethod::Get);
    assert_eq!(sends[0].1.url, COMPLETE_URL);
    assert_eq!(sends[0].1.body, None);
}

#[test]
fn method_choice_non_empty_data_sends_a_post() {
    let mut run = harness();
    run.deps.config.transformed = Ok((COMPLETE_URL.to_owned(), operation()));
    let params = Params::new();
    run.sync(Call::new(&params))
        .expect("the status call succeeds");
    let sends = sync_sends(&run.events());
    assert_eq!(sends[0].1.method, StatusMethod::Post);
    assert_eq!(sends[0].1.body, Some(operation()));
}

#[test]
fn method_choice_one_entry_data_sends_a_post() {
    let mut run = harness();
    let data = Params::from([("flag".to_owned(), json("true"))]);
    run.deps.config.transformed = Ok((COMPLETE_URL.to_owned(), data.clone()));
    let params = Params::new();
    run.sync(Call::new(&params))
        .expect("the status call succeeds");
    let sends = sync_sends(&run.events());
    assert_eq!(sends[0].1.method, StatusMethod::Post);
    assert_eq!(sends[0].1.body, Some(data));
}

#[tokio::test]
async fn method_choice_the_async_call_sends_the_same_request() {
    let mut run = harness();
    run.deps.config.transformed = Ok((COMPLETE_URL.to_owned(), operation()));
    let params = Params::new();
    run.asynchronous(Call::new(&params))
        .await
        .expect("the status call succeeds");
    let sends = async_sends(&run.events());
    assert_eq!(sends[0].1.method, StatusMethod::Post);
    assert_eq!(sends[0].1.url, COMPLETE_URL);
    assert_eq!(sends[0].1.body, Some(operation()));
}

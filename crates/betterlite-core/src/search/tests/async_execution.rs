#![cfg(test)]

use std::time::Duration;

use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;
use betterlite_models::decisions::Json;

use super::support::{ANSWER, Harness, Recorder, Registry, json, request};

#[tokio::test(flavor = "current_thread")]
async fn async_execution_keeps_each_call_in_its_own_logging_state() {
    let first_server = serve(reply("200 OK", &[], ANSWER), Duration::from_millis(200))
        .expect("the fixture serves");
    let second_server =
        serve(reply("200 OK", &[], ANSWER), Duration::ZERO).expect("the fixture serves");
    let registry = Registry::new(Recorder::default());
    let mut first = request(r#""rust""#, &format!("http://{}", first_server.address));
    first.kwargs = serde_json::from_str(r#"{"litellm_call_id":"first","lang":"en"}"#).unwrap();
    let mut second = request(r#""go""#, &format!("http://{}", second_server.address));
    second.kwargs = serde_json::from_str(r#"{"litellm_call_id":"second","lang":"de"}"#).unwrap();
    let mut first_harness = Harness::new();
    let mut second_harness = Harness::new();
    let (first_result, second_result) = tokio::join!(
        first_harness.search(&first, &registry),
        second_harness.search(&second, &registry)
    );
    first_result.unwrap();
    second_result.unwrap();
    assert_eq!(
        first_harness.state.litellm_params.get("litellm_call_id"),
        Some(&Json::Text("first".to_owned()))
    );
    assert_eq!(
        Json::Object(first_harness.state.optional_params.clone()),
        json(r#"{"lang":"en"}"#)
    );
    assert_eq!(
        second_harness.state.litellm_params.get("litellm_call_id"),
        Some(&Json::Text("second".to_owned()))
    );
    assert_eq!(
        Json::Object(second_harness.state.optional_params.clone()),
        json(r#"{"lang":"de"}"#)
    );
    assert_eq!(
        first_server.finish().expect("the fixture answers").body,
        r#""rust""#
    );
    assert_eq!(
        second_server.finish().expect("the fixture answers").body,
        r#""go""#
    );
}

#[tokio::test(flavor = "current_thread")]
async fn async_execution_returns_the_response_on_the_calling_task() {
    let served = serve(reply("200 OK", &[], ANSWER), Duration::ZERO).expect("the fixture serves");
    let registry = Registry::new(Recorder::default());
    let mut harness = Harness::new();
    let response = harness
        .search(
            &request(r#""rust""#, &format!("http://{}", served.address)),
            &registry,
        )
        .await
        .unwrap();
    assert_eq!(response.results.len(), 1);
    assert_eq!(harness.state.model, "brave/search");
    served.finish().expect("the fixture answers");
}

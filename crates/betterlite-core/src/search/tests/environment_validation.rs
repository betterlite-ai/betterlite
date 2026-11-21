#![cfg(test)]

use std::time::Duration;

use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;
use betterlite_openai::openai_base_config::{Headers, Object};

use super::support::{ANSWER, Call, Harness, Recorder, Registry, request};

#[tokio::test]
async fn environment_validation_gets_the_caller_headers_key_and_base() {
    let served = serve(reply("200 OK", &[], ANSWER), Duration::ZERO).expect("the fixture serves");
    let base = format!("http://{}", served.address);
    let recorder = Recorder::default();
    let registry = Registry::new(recorder.clone());
    let mut search = request(r#""rust""#, &base);
    search.extra_headers = Headers::from([("x-trace".to_owned(), "abc".to_owned())]);
    let mut harness = Harness::new();
    harness.search(&search, &registry).await.unwrap();
    assert_eq!(
        recorder.calls().first(),
        Some(&Call::Validate {
            headers: Headers::from([("x-trace".to_owned(), "abc".to_owned())]),
            api_key: Some("sk-search".to_owned()),
            api_base: Some(base),
        })
    );
    let sent = served
        .finish()
        .expect("the fixture answers")
        .head
        .to_lowercase();
    assert!(sent.contains("x-trace: abc"));
    assert!(sent.contains("x-validated: yes"));
}

#[tokio::test]
async fn environment_validation_starts_from_no_headers_without_a_key_or_base() {
    let recorder = Recorder::new(true, Object::new());
    let registry = Registry::new(recorder.clone());
    let mut search = request(r#""rust""#, "http://127.0.0.1:9");
    search.api_key = None;
    search.api_base = None;
    let mut harness = Harness::new();
    harness.search(&search, &registry).await.unwrap_err();
    assert_eq!(
        recorder.calls().first(),
        Some(&Call::Validate {
            headers: Headers::new(),
            api_key: None,
            api_base: None,
        })
    );
}

#[tokio::test]
async fn environment_validation_headers_are_the_ones_the_handler_sends() {
    let served = serve(reply("200 OK", &[], ANSWER), Duration::ZERO).expect("the fixture serves");
    let recorder = Recorder::default();
    let registry = Registry::new(recorder.clone());
    let mut harness = Harness::new();
    harness
        .search(
            &request(r#""rust""#, &format!("http://{}", served.address)),
            &registry,
        )
        .await
        .unwrap();
    let validated = Headers::from([("x-validated".to_owned(), "yes".to_owned())]);
    let second = recorder
        .calls()
        .into_iter()
        .filter(|call| matches!(call, Call::Validate { .. }))
        .nth(1);
    assert!(matches!(
        second,
        Some(Call::Validate { headers, .. }) if headers == validated
    ));
    served.finish().expect("the fixture answers");
}

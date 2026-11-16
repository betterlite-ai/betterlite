#![cfg(test)]

use reqwest::blocking::Client as BlockingClient;
use reqwest::blocking::ClientBuilder as BlockingClientBuilder;
use reqwest::blocking::Response as BlockingResponse;
use reqwest::{Client, Response};
use rstest::rstest;

use crate::interactions::base::Logging;
use crate::interactions::base::tests::support as base;
use crate::interactions::create::tests::support::{Fixture, FixtureError, provider_error};
use crate::interactions::streaming::{create_async_iterator, create_sync_iterator};
use crate::proxy::base_builder;

type Sent = (base::Serving, BlockingClient, BlockingResponse);

type SentAsync = (base::Serving, Client, Response);

fn answering(status: &str, headers: &[(&str, &str)], body: &str) -> (base::Serving, String) {
    let serving = base::serve_once(base::reply(status, headers, body), None);
    let url = format!("http://{}", serving.address);
    (serving, url)
}

fn sent(status: &str, headers: &[(&str, &str)], body: &str) -> Sent {
    let (serving, url) = answering(status, headers, body);
    let client = BlockingClientBuilder::new().build().unwrap();
    let response = client.get(url).send().unwrap();
    (serving, client, response)
}

async fn sent_async(status: &str, headers: &[(&str, &str)], body: &str) -> SentAsync {
    let (serving, url) = answering(status, headers, body);
    let client = base_builder().build().unwrap();
    let response = client.get(url).send().await.unwrap();
    (serving, client, response)
}

#[rstest]
#[case::ok("200 OK", 200, "hello")]
#[case::no_content("204 No Content", 204, "")]
#[case::last_ok("299 Custom", 299, "hello")]
fn create_sync_iterator_accepts_a_2xx_status(
    #[case] status: &str,
    #[case] code: u16,
    #[case] body: &str,
) {
    let (serving, _client, response) = sent(status, &[], body);
    let fixture = Fixture::new();

    let seen =
        create_sync_iterator(response, Some("m"), &mut Logging::default(), &fixture).unwrap();

    assert_eq!(seen, body);
    assert!(fixture.calls().contains(&format!("sync_stream|{code}|m")));
    serving.request.join().unwrap();
}

#[rstest]
#[case::redirect("300 Multiple Choices", 300)]
#[case::missing("404 Not Found", 404)]
fn create_sync_iterator_refuses_a_status_outside_2xx(#[case] status: &str, #[case] code: u16) {
    let (serving, _client, response) = sent(status, &[("retry-after", "5")], "missing");
    let fixture = Fixture::new();

    let error = create_sync_iterator(response, None, &mut Logging::default(), &fixture);

    assert_eq!(provider_error(error), FixtureError::Refused(code));
    let calls = fixture.calls();
    assert!(
        calls.contains(&format!("error|{code}|missing|5")),
        "{calls:?}"
    );
    assert!(calls.iter().all(|call| !call.starts_with("sync_stream|")));
    serving.request.join().unwrap();
}

#[test]
fn create_sync_iterator_leaves_a_config_error_as_a_provider_error() {
    let (serving, _client, response) = sent("200 OK", &[], "{}");
    let fixture = Fixture {
        refuse_stream: true,
        ..Fixture::new()
    };
    let mut logging = Logging::default();

    let error = create_sync_iterator(response, None, &mut logging, &fixture);

    assert_eq!(provider_error(error), FixtureError::Stream);
    assert!(logging.model_call_details.contains_key("streamed"));
    serving.request.join().unwrap();
}

#[rstest]
#[case::ok("200 OK", 200)]
#[case::last_ok("299 Custom", 299)]
#[tokio::test]
async fn create_async_iterator_accepts_a_2xx_status(#[case] status: &str, #[case] code: u16) {
    let (serving, client, response) = sent_async(status, &[], "hello").await;
    let fixture = Fixture::new();

    let seen = create_async_iterator(
        &client,
        response,
        Some("m"),
        &mut Logging::default(),
        &fixture,
    )
    .await
    .unwrap();

    // the factory leaves the body unread, so the iterator can still read it here
    assert_eq!(seen.response.text().await.unwrap(), "hello");
    assert!(fixture.calls().contains(&format!("async_stream|{code}|m")));
    serving.request.join().unwrap();
}

#[rstest]
#[case::redirect("300 Multiple Choices", 300)]
#[case::missing("404 Not Found", 404)]
#[tokio::test]
async fn create_async_iterator_refuses_a_status_outside_2xx(
    #[case] status: &str,
    #[case] code: u16,
) {
    let (serving, client, response) = sent_async(status, &[("retry-after", "5")], "missing").await;
    let fixture = Fixture::new();
    let mut logging = Logging::default();

    let error = create_async_iterator(&client, response, None, &mut logging, &fixture);

    assert_eq!(provider_error(error.await), FixtureError::Refused(code));
    let calls = fixture.calls();
    assert!(
        calls.contains(&format!("error|{code}|missing|5")),
        "{calls:?}"
    );
    assert!(calls.iter().all(|call| !call.starts_with("async_stream|")));
    serving.request.join().unwrap();
}

#[tokio::test]
async fn create_async_iterator_leaves_a_config_error_as_a_provider_error() {
    let (serving, client, response) = sent_async("200 OK", &[], "{}").await;
    let fixture = Fixture {
        refuse_stream: true,
        ..Fixture::new()
    };
    let mut logging = Logging::default();

    let error = create_async_iterator(&client, response, None, &mut logging, &fixture);

    assert_eq!(provider_error(error.await), FixtureError::Stream);
    assert!(logging.model_call_details.contains_key("streamed"));
    serving.request.join().unwrap();
}

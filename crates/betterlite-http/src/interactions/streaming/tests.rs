#![cfg(test)]

use std::str::from_utf8;

use betterlite_models::controls::json_value::JsonValue;
use reqwest::blocking::ClientBuilder as BlockingClientBuilder;
use rstest::rstest;

use crate::interactions::base::tests::support as base;
use crate::interactions::base::{
    ErrorClasses, Logging, RequestFailure, ResponseHeaders, UpstreamResponse,
};
use crate::interactions::streaming::{
    StreamError, StreamingConfig, create_async_iterator, create_sync_iterator,
};
use crate::proxy::StreamResponse;
use crate::proxy::base_builder;

#[derive(Debug, thiserror::Error)]
enum FakeError {
    #[error("refused with status {status_code}")]
    Refused { status_code: u16 },
    #[error("the stream is refused")]
    Stream,
}

struct FakeConfig {
    refuse: bool,
}

#[derive(Debug)]
struct SyncSeen {
    model: Option<String>,
    body: String,
    test_header: Option<String>,
}

struct AsyncSeen {
    model: Option<String>,
    source: StreamResponse,
}

impl ErrorClasses for FakeConfig {
    type Error = FakeError;

    fn get_error_class(
        &self,
        _message: &str,
        status_code: u16,
        _headers: &ResponseHeaders,
    ) -> FakeError {
        FakeError::Refused { status_code }
    }
}

impl StreamingConfig for FakeConfig {
    type SyncStream = SyncSeen;

    type AsyncStream = AsyncSeen;

    fn create_sync_stream_iterator(
        &self,
        response: UpstreamResponse,
        model: Option<&str>,
        logging: &mut Logging,
    ) -> Result<SyncSeen, FakeError> {
        logging
            .model_call_details
            .insert("sync".to_owned(), JsonValue::Bool(true));
        if self.refuse {
            return Err(FakeError::Stream);
        }
        Ok(SyncSeen {
            model: model.map(str::to_owned),
            body: from_utf8(&response.body).unwrap_or_default().to_owned(),
            test_header: response.headers.get("x-test").cloned(),
        })
    }

    fn create_async_stream_iterator(
        &self,
        response: StreamResponse,
        model: Option<&str>,
        logging: &mut Logging,
    ) -> Result<AsyncSeen, FakeError> {
        logging
            .model_call_details
            .insert("async".to_owned(), JsonValue::Bool(true));
        if self.refuse {
            return Err(FakeError::Stream);
        }
        Ok(AsyncSeen {
            model: model.map(str::to_owned),
            source: response,
        })
    }
}

#[test]
fn create_sync_iterator_hands_the_buffered_response_model_and_headers_to_the_config() {
    let serving = base::serve_once(base::reply("200 OK", &[("x-test", "yes")], "hello"), None);
    let client = BlockingClientBuilder::new().build().unwrap();
    let response = client
        .get(format!("http://{}", serving.address))
        .send()
        .unwrap();
    let config = FakeConfig { refuse: false };
    let mut logging = Logging::default();

    let seen = create_sync_iterator(response, Some("m"), &mut logging, &config).unwrap();

    assert_eq!(seen.model.as_deref(), Some("m"));
    assert_eq!(seen.body, "hello");
    assert_eq!(seen.test_header.as_deref(), Some("yes"));
    assert_eq!(
        logging.model_call_details.get("sync"),
        Some(&JsonValue::Bool(true))
    );
    serving.request.join().unwrap();
}

#[rstest]
#[case::ok("200 OK", "hello")]
#[case::no_content("204 No Content", "")]
#[case::last_ok("299 Custom", "hello")]
fn create_sync_iterator_accepts_a_2xx_status(#[case] status: &str, #[case] body: &str) {
    let serving = base::serve_once(base::reply(status, &[], body), None);
    let client = BlockingClientBuilder::new().build().unwrap();
    let response = client
        .get(format!("http://{}", serving.address))
        .send()
        .unwrap();
    let config = FakeConfig { refuse: false };
    let mut logging = Logging::default();

    let seen = create_sync_iterator(response, None, &mut logging, &config).unwrap();

    assert_eq!(seen.body, body);
    serving.request.join().unwrap();
}

#[rstest]
#[case::redirect("300 Multiple Choices", 300)]
#[case::missing("404 Not Found", 404)]
fn create_sync_iterator_refuses_a_status_outside_2xx(#[case] status: &str, #[case] code: u16) {
    let serving = base::serve_once(
        base::reply(status, &[("retry-after", "5")], "missing"),
        None,
    );
    let client = BlockingClientBuilder::new().build().unwrap();
    let response = client
        .get(format!("http://{}", serving.address))
        .send()
        .unwrap();
    let config = FakeConfig { refuse: false };
    let mut logging = Logging::default();

    let error = create_sync_iterator(response, None, &mut logging, &config)
        .err()
        .unwrap();

    match error {
        StreamError::Read(RequestFailure::Status {
            status_code,
            headers,
            body,
        }) => {
            assert_eq!(status_code, code);
            assert_eq!(body, "missing");
            assert_eq!(headers.get("retry-after").map(String::as_str), Some("5"));
        }
        other => panic!("expected a status failure, got {other:?}"),
    }
    assert!(logging.model_call_details.get("sync").is_none());
    serving.request.join().unwrap();
}

#[test]
fn create_sync_iterator_leaves_a_config_error_as_a_provider_error() {
    let serving = base::serve_once(base::reply("200 OK", &[], "{}"), None);
    let client = BlockingClientBuilder::new().build().unwrap();
    let response = client
        .get(format!("http://{}", serving.address))
        .send()
        .unwrap();
    let config = FakeConfig { refuse: true };
    let mut logging = Logging::default();

    let error = create_sync_iterator(response, None, &mut logging, &config)
        .err()
        .unwrap();

    assert!(matches!(error, StreamError::Provider(FakeError::Stream)));
    serving.request.join().unwrap();
}

#[tokio::test]
async fn create_async_iterator_hands_the_open_response_and_model_to_the_config() {
    let serving = base::serve_once(base::reply("200 OK", &[], "hello"), None);
    let client = base_builder().build().unwrap();
    let response = client
        .get(format!("http://{}", serving.address))
        .send()
        .await
        .unwrap();
    let config = FakeConfig { refuse: false };
    let mut logging = Logging::default();

    let seen = create_async_iterator(&client, response, Some("m"), &mut logging, &config)
        .await
        .unwrap();

    assert_eq!(seen.model.as_deref(), Some("m"));
    // the factory leaves the body unread, so the iterator can still read it here
    assert_eq!(seen.source.response.text().await.unwrap(), "hello");
    assert_eq!(
        logging.model_call_details.get("async"),
        Some(&JsonValue::Bool(true))
    );
    serving.request.join().unwrap();
}

#[rstest]
#[case::ok("200 OK")]
#[case::last_ok("299 Custom")]
#[tokio::test]
async fn create_async_iterator_accepts_a_2xx_status(#[case] status: &str) {
    let serving = base::serve_once(base::reply(status, &[], "hello"), None);
    let client = base_builder().build().unwrap();
    let response = client
        .get(format!("http://{}", serving.address))
        .send()
        .await
        .unwrap();
    let config = FakeConfig { refuse: false };
    let mut logging = Logging::default();

    let seen = create_async_iterator(&client, response, None, &mut logging, &config)
        .await
        .unwrap();

    assert_eq!(seen.source.response.text().await.unwrap(), "hello");
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
    let serving = base::serve_once(
        base::reply(status, &[("retry-after", "5")], "missing"),
        None,
    );
    let client = base_builder().build().unwrap();
    let response = client
        .get(format!("http://{}", serving.address))
        .send()
        .await
        .unwrap();
    let config = FakeConfig { refuse: false };
    let mut logging = Logging::default();

    let error = create_async_iterator(&client, response, None, &mut logging, &config)
        .await
        .err()
        .unwrap();

    match error {
        StreamError::Read(RequestFailure::Status {
            status_code,
            headers,
            body,
        }) => {
            assert_eq!(status_code, code);
            assert_eq!(body, "missing");
            assert_eq!(headers.get("retry-after").map(String::as_str), Some("5"));
        }
        other => panic!("expected a status failure, got {other:?}"),
    }
    assert!(logging.model_call_details.get("async").is_none());
    serving.request.join().unwrap();
}

#[tokio::test]
async fn create_async_iterator_leaves_a_config_error_as_a_provider_error() {
    let serving = base::serve_once(base::reply("200 OK", &[], "{}"), None);
    let client = base_builder().build().unwrap();
    let response = client
        .get(format!("http://{}", serving.address))
        .send()
        .await
        .unwrap();
    let config = FakeConfig { refuse: true };
    let mut logging = Logging::default();

    let error = create_async_iterator(&client, response, None, &mut logging, &config)
        .await
        .err()
        .unwrap();

    assert!(matches!(error, StreamError::Provider(FakeError::Stream)));
    serving.request.join().unwrap();
}

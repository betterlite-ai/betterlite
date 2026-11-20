#![cfg(test)]

mod errors;
mod list;
mod recording;
mod remix;
mod support;

use std::time::Duration;

use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;
use http::Method;
use rstest::rstest;

use self::errors::{
    CLOSED, DEADLINE, ErrorCase, LIST, PROMPT, REMIX, assert_error, assert_field,
    assert_outside_transform, assert_prepare, assert_timeout, expected_headers, provider,
    timed_remix,
};
use self::recording::{RecordingConfig, Step};
use self::support::{
    Bench, CallKind, CallMode, Events, Fixture, IMPATIENT, Reply, Setup, Source, Wire, failing,
    fake_bench, json_params, proxied_bench, run, transport,
};
use crate::api::vector_store_http_write::error::timeout_facts;
use crate::api::vector_store_http_write::service::ClientKind;
use crate::api::video_remix_list_transport::error::{Operation, VideoRemixListTransportError};

#[rstest]
#[case::sync_supplied_client(remix::RemixCase::SyncSuppliedClient)]
#[case::sync_pooled_client(remix::RemixCase::SyncPooledClient)]
#[case::async_pooled_client(remix::RemixCase::AsyncPooledClient)]
#[case::async_supplied_client(remix::RemixCase::AsyncSuppliedClient)]
#[case::async_real_server(remix::RemixCase::AsyncRealServer)]
#[case::api_key_argument(remix::RemixCase::ApiKeyArgument)]
#[case::pre_call(remix::RemixCase::PreCall)]
#[case::is_async_hand_off(remix::RemixCase::IsAsyncHandOff)]
#[case::response_unchanged(remix::RemixCase::ResponseUnchanged)]
#[tokio::test]
async fn remix_transport_behavior(#[case] case: remix::RemixCase) {
    remix::remix_case(case).await;
}

#[rstest]
#[case::api_key_ignored_from_params(list::ListCase::ApiKeyIgnoredFromParams)]
#[case::api_key_argument(list::ListCase::ApiKeyArgument)]
#[case::get_query_and_no_timeout(list::ListCase::GetQueryAndNoTimeout)]
#[case::async_supplied_client(list::ListCase::AsyncSuppliedClient)]
#[case::real_get(list::ListCase::RealGet)]
#[case::sync_runs_to_completion(list::ListCase::SyncRunsToCompletion)]
#[case::sync_forwards_async_client(list::ListCase::SyncForwardsAsyncClient)]
#[case::hand_off_forwards_api_key(list::ListCase::HandOffForwardsApiKey)]
#[case::running_runtime(list::ListCase::RunningRuntime)]
#[test]
fn list_transport_behavior(#[case] case: list::ListCase) {
    list::list_case(case);
}

#[rstest]
#[case::remix_status(ErrorCase::RemixStatus)]
#[case::remix_status_empty_body(ErrorCase::RemixStatusEmptyBody)]
#[case::remix_status_binary_body(ErrorCase::RemixStatusBinaryBody)]
#[case::remix_send_failure(ErrorCase::RemixSendFailure)]
#[case::remix_timeout(ErrorCase::RemixTimeout)]
#[case::remix_caller_timeout(ErrorCase::RemixCallerTimeout)]
#[case::sync_timeout_text(ErrorCase::SyncTimeoutText)]
#[case::remix_response_conversion(ErrorCase::RemixResponseConversion)]
#[case::remix_request_conversion(ErrorCase::RemixRequestConversion)]
#[case::remix_url_failure(ErrorCase::RemixUrlFailure)]
#[case::list_status(ErrorCase::ListStatus)]
#[case::list_send_failure(ErrorCase::ListSendFailure)]
#[case::list_timeout(ErrorCase::ListTimeout)]
#[case::list_response_conversion(ErrorCase::ListResponseConversion)]
#[case::list_query_object(ErrorCase::ListQueryObject)]
#[case::list_query_nested(ErrorCase::ListQueryNested)]
#[case::unknown_provider(ErrorCase::UnknownProvider)]
#[case::pool_failure(ErrorCase::PoolFailure)]
#[case::api_base_type(ErrorCase::ApiBaseType)]
#[case::api_key_type(ErrorCase::ApiKeyType)]
#[case::ssl_verify_type(ErrorCase::SslVerifyType)]
#[case::list_ssl_verify_type(ErrorCase::ListSslVerifyType)]
#[case::list_api_base_type(ErrorCase::ListApiBaseType)]
#[tokio::test]
async fn transport_error_behavior(#[case] case: ErrorCase) {
    match case {
        ErrorCase::RemixStatus => {
            let body = r#"{"error":{"message":"slow down"}}"#;
            let answered = Reply::json(429, body)
                .with_header("retry-after", "7")
                .with_header("content-encoding", "gzip")
                .with_header("content-length", "999");
            let (mut bench, _) = fake_bench(REMIX, answered, RecordingConfig::new);
            let params = json_params("{}");
            let setup = Setup::new("openai", &params).video("video_a", PROMPT);
            let error = run(&mut bench, setup, CallKind::Remix, CallMode::Asynchronous)
                .await
                .expect_err("a 429 is the provider's error");
            assert_error(&bench, &error, 429, body, false);
            let length = body.len().to_string();
            assert_eq!(
                bench.config.errors()[0].headers,
                expected_headers(&[("content-length", length.as_str()), ("retry-after", "7")])
            );
            assert_outside_transform(&bench, "remix_response");
        }
        ErrorCase::RemixStatusEmptyBody => {
            let (mut bench, _) = fake_bench(REMIX, Reply::json(400, ""), RecordingConfig::new);
            let params = json_params("{}");
            let setup = Setup::new("openai", &params).video("video_a", PROMPT);
            let error = run(&mut bench, setup, CallKind::Remix, CallMode::Asynchronous)
                .await
                .expect_err("a 400 is the provider's error");
            assert_error(&bench, &error, 400, "", false);
            assert!(bench.config.errors()[0].headers.is_empty());
        }
        ErrorCase::RemixStatusBinaryBody => {
            let answered = Reply::bytes(400, vec![0xff, 0xfe]);
            let (mut bench, _) = fake_bench(REMIX, answered, RecordingConfig::new);
            let params = json_params("{}");
            let setup = Setup::new("openai", &params).video("video_a", PROMPT);
            let error = run(&mut bench, setup, CallKind::Remix, CallMode::Asynchronous)
                .await
                .expect_err("the body is named by its byte count");
            assert_error(&bench, &error, 400, "2 bytes that are not UTF-8", false);
            assert_eq!(
                bench.config.errors()[0].headers,
                expected_headers(&[("content-length", "2")])
            );
        }
        ErrorCase::RemixSendFailure => {
            let (mut bench, wire) = fake_bench(REMIX, Reply::json(200, "{}"), RecordingConfig::new);
            failing(&wire, TransportError::Closed);
            let params = json_params("{}");
            let setup = Setup::new("openai", &params).video("video_a", PROMPT);
            let error = run(&mut bench, setup, CallKind::Remix, CallMode::Asynchronous)
                .await
                .expect_err("a transport failure is an error");
            assert_error(&bench, &error, 500, CLOSED, true);
            assert_eq!(wire.requests().len(), 1);
            assert_eq!(wire.requests()[0].method, Method::POST);
            assert_outside_transform(&bench, "remix_response");
        }
        ErrorCase::RemixTimeout => {
            let (bench, error) = timed_remix(None).await;
            assert_timeout(&bench, &error, "0.1");
        }
        ErrorCase::RemixCallerTimeout => {
            let (bench, error) = timed_remix(Some(Duration::from_millis(50))).await;
            assert_timeout(&bench, &error, "0.05");
        }
        ErrorCase::SyncTimeoutText => {
            let sync_none = timeout_facts(ClientKind::Sync, None, Duration::from_millis(50));
            assert_eq!(
                sync_none.text.as_deref(),
                Some("litellm.Timeout: Connection timed out after None seconds.")
            );
            let sync_whole = timeout_facts(
                ClientKind::Sync,
                Some(Duration::from_secs(2)),
                Duration::from_millis(50),
            );
            assert_eq!(
                sync_whole.text.as_deref(),
                Some("litellm.Timeout: Connection timed out after 2 seconds.")
            );
            let asynchronous = timeout_facts(
                ClientKind::Asynchronous,
                Some(Duration::from_secs(2)),
                Duration::from_millis(250),
            );
            assert_eq!(
                asynchronous.text.as_deref(),
                Some(
                    "litellm.Timeout: Connection timed out. Timeout passed=2, time taken=0.250 seconds"
                )
            );
        }
        ErrorCase::RemixResponseConversion => {
            let configure =
                |events: &Events| RecordingConfig::new(events).failing(Step::RemixResponse);
            let answered = Reply::json(200, "not json").with_header("x-request-id", "req-1");
            let (mut bench, _) = fake_bench(REMIX, answered, configure);
            let params = json_params("{}");
            let setup = Setup::new("openai", &params).video("video_a", PROMPT);
            let error = run(&mut bench, setup, CallKind::Remix, CallMode::Asynchronous)
                .await
                .expect_err("a bad provider body is converted");
            assert_error(&bench, &error, 200, "not json", false);
            assert_eq!(
                bench.config.errors()[0].headers,
                expected_headers(&[("content-length", "8"), ("x-request-id", "req-1")])
            );
            assert!(
                bench
                    .config
                    .events()
                    .contains(&String::from("remix_response"))
            );
        }
        ErrorCase::RemixRequestConversion => {
            let configure =
                |events: &Events| RecordingConfig::new(events).failing(Step::RemixRequest);
            let (mut bench, wire) = fake_bench(REMIX, Reply::json(200, "{}"), configure);
            let params = json_params("{}");
            let setup = Setup::new("openai", &params).video("video_a", PROMPT);
            let error = run(&mut bench, setup, CallKind::Remix, CallMode::Asynchronous)
                .await
                .expect_err("a request transform failure is a prepare failure");
            assert_prepare(&error, Operation::Remix, Step::RemixRequest);
            assert!(wire.requests().is_empty());
            assert_eq!(bench.config.events(), ["validate", "url", "remix_request"]);
        }
        ErrorCase::RemixUrlFailure => {
            let configure = |events: &Events| RecordingConfig::new(events).failing(Step::Url);
            let (mut bench, wire) = fake_bench(REMIX, Reply::json(200, "{}"), configure);
            let params = json_params("{}");
            let setup = Setup::new("openai", &params).video("video_a", PROMPT);
            let error = run(&mut bench, setup, CallKind::Remix, CallMode::Asynchronous)
                .await
                .expect_err("a URL failure is a prepare failure");
            assert_prepare(&error, Operation::Remix, Step::Url);
            assert!(wire.requests().is_empty());
            assert_eq!(bench.config.events(), ["validate", "url"]);
        }
        ErrorCase::ListStatus => {
            let body = r#"{"error":"bad"}"#;
            let (mut bench, _) = fake_bench(LIST, Reply::json(400, body), RecordingConfig::new);
            let params = json_params("{}");
            let setup = Setup::new("openai", &params);
            let error = run(&mut bench, setup, CallKind::List, CallMode::Asynchronous)
                .await
                .expect_err("a 400 is the provider's error");
            assert_error(&bench, &error, 400, body, false);
            let length = body.len().to_string();
            assert_eq!(
                bench.config.errors()[0].headers,
                expected_headers(&[("content-length", length.as_str())])
            );
            assert_outside_transform(&bench, "list_response");
        }
        ErrorCase::ListSendFailure => {
            let (mut bench, wire) = fake_bench(LIST, Reply::json(200, "{}"), RecordingConfig::new);
            failing(&wire, TransportError::Closed);
            let params = json_params("{}");
            let setup = Setup::new("openai", &params);
            let error = run(&mut bench, setup, CallKind::List, CallMode::Asynchronous)
                .await
                .expect_err("a transport failure is an error");
            assert_error(&bench, &error, 500, CLOSED, true);
            assert_eq!(wire.requests().len(), 1);
            assert_eq!(wire.requests()[0].method, Method::GET);
            assert_outside_transform(&bench, "list_response");
        }
        ErrorCase::ListTimeout => {
            let serving = serve(
                reply("200 fixture", &[("content-type", "application/json")], "{}"),
                Duration::from_millis(300),
            )
            .expect("the fixture serves");
            let address = serving.address;
            let configure = move |events: &Events| {
                RecordingConfig::new(events)
                    .with_base(&format!("http://{address}/v1"))
                    .with_list_url(&format!("http://{address}/v1/videos"))
            };
            let mut bench = proxied_bench(LIST, configure, IMPATIENT);
            let params = json_params("{}");
            let setup = Setup::new("openai", &params);
            let error = run(&mut bench, setup, CallKind::List, CallMode::Asynchronous)
                .await
                .expect_err("the read timeout is an error");
            serving.finish().expect("the fixture finishes");
            assert_error(&bench, &error, 500, DEADLINE, true);
            assert!(!provider(&error).message.starts_with("litellm.Timeout"));
        }
        ErrorCase::ListResponseConversion => {
            let configure =
                |events: &Events| RecordingConfig::new(events).failing(Step::ListResponse);
            let answered = Reply::json(200, "not json").with_header("x-request-id", "req-1");
            let (mut bench, _) = fake_bench(LIST, answered, configure);
            let params = json_params("{}");
            let setup = Setup::new("openai", &params);
            let error = run(&mut bench, setup, CallKind::List, CallMode::Asynchronous)
                .await
                .expect_err("a bad provider page is converted");
            assert_error(&bench, &error, 200, "not json", false);
            assert_eq!(
                bench.config.errors()[0].headers,
                expected_headers(&[("content-length", "8"), ("x-request-id", "req-1")])
            );
            assert!(
                bench
                    .config
                    .events()
                    .contains(&String::from("list_response"))
            );
        }
        ErrorCase::ListQueryObject => {
            let configure = |events: &Events| {
                RecordingConfig::new(events)
                    .with_list_params(json_params(r#"{"filter":{"tier":1}}"#))
            };
            let (mut bench, wire) = fake_bench(LIST, Reply::json(200, "{}"), configure);
            let params = json_params("{}");
            let setup = Setup::new("openai", &params);
            let error = run(&mut bench, setup, CallKind::List, CallMode::Asynchronous)
                .await
                .expect_err("an object query value has no form");
            assert_error(
                &bench,
                &error,
                500,
                "the filter query value is not a scalar",
                true,
            );
            assert!(wire.requests().is_empty());
            assert!(!bench.config.events().contains(&String::from("send")));
        }
        ErrorCase::ListQueryNested => {
            let configure = |events: &Events| {
                RecordingConfig::new(events).with_list_params(json_params(r#"{"tag":[["a"]]}"#))
            };
            let (mut bench, wire) = fake_bench(LIST, Reply::json(200, "{}"), configure);
            let params = json_params("{}");
            let setup = Setup::new("openai", &params);
            let error = run(&mut bench, setup, CallKind::List, CallMode::Asynchronous)
                .await
                .expect_err("a nested list has no form");
            assert_error(
                &bench,
                &error,
                500,
                "the tag query value is not a scalar",
                true,
            );
            assert!(wire.requests().is_empty());
        }
        ErrorCase::UnknownProvider => {
            let (mut bench, wire) = fake_bench(REMIX, Reply::json(200, "{}"), RecordingConfig::new);
            let params = json_params("{}");
            let setup = Setup::new("nope", &params).video("video_a", PROMPT);
            let error = run(&mut bench, setup, CallKind::Remix, CallMode::Asynchronous)
                .await
                .expect_err("an unknown provider fails before the try block");
            assert!(matches!(
                error,
                VideoRemixListTransportError::UnknownProvider { .. }
            ));
            assert_eq!(
                error.to_string(),
                "\"nope\" is not a provider betterlite knows"
            );
            assert!(bench.config.events().is_empty());
            assert!(wire.requests().is_empty());
            assert!(bench.transport.source.async_calls().is_empty());
        }
        ErrorCase::PoolFailure => {
            let events = Events::new();
            let config = RecordingConfig::new(&events);
            let wire = Wire::replying(Reply::json(200, "{}"), &events);
            let fixture = Fixture::new(REMIX, &events);
            let mut bench = Bench {
                events,
                fixture,
                config,
                transport: transport(Source::failing(wire.clone())),
            };
            let params = json_params("{}");
            let setup = Setup::new("openai", &params).video("video_a", PROMPT);
            let error = run(&mut bench, setup, CallKind::Remix, CallMode::Asynchronous)
                .await
                .expect_err("a pool failure is not a provider error");
            assert!(matches!(error, VideoRemixListTransportError::Client { .. }));
            assert_eq!(
                error.to_string(),
                "the video HTTP client could not be built"
            );
            assert!(bench.config.events().is_empty());
            assert!(wire.requests().is_empty());
        }
        ErrorCase::ApiBaseType => {
            let (mut bench, wire) = fake_bench(REMIX, Reply::json(200, "{}"), RecordingConfig::new);
            let params = json_params(r#"{"api_base":1}"#);
            let setup = Setup::new("openai", &params).video("video_a", PROMPT);
            let error = run(&mut bench, setup, CallKind::Remix, CallMode::Asynchronous)
                .await
                .expect_err("a number api_base is a field error");
            assert_field(&error, "litellm_params.api_base must be a string");
            assert_eq!(bench.config.events(), ["validate"]);
            assert!(wire.requests().is_empty());
        }
        ErrorCase::ApiKeyType => {
            let (mut bench, _) = fake_bench(REMIX, Reply::json(200, "{}"), RecordingConfig::new);
            let params = json_params(r#"{"api_key":7}"#);
            let setup = Setup::new("openai", &params).video("video_a", PROMPT);
            let error = run(&mut bench, setup, CallKind::Remix, CallMode::Asynchronous)
                .await
                .expect_err("a number api_key is a field error");
            assert_field(&error, "litellm_params.api_key must be a string");
            assert!(bench.config.events().is_empty());
        }
        ErrorCase::SslVerifyType => {
            let (mut bench, wire) = fake_bench(REMIX, Reply::json(200, "{}"), RecordingConfig::new);
            let params = json_params(r#"{"ssl_verify":"yes"}"#);
            let setup = Setup::new("openai", &params).video("video_a", PROMPT);
            let error = run(&mut bench, setup, CallKind::Remix, CallMode::Asynchronous)
                .await
                .expect_err("a text ssl_verify is a field error");
            assert_field(&error, "litellm_params.ssl_verify must be a boolean");
            assert!(bench.config.events().is_empty());
            assert!(wire.requests().is_empty());
            assert!(bench.transport.source.async_calls().is_empty());
        }
        ErrorCase::ListSslVerifyType => {
            let (mut bench, _) = fake_bench(LIST, Reply::json(200, "{}"), RecordingConfig::new);
            let params = json_params(r#"{"ssl_verify":"yes"}"#);
            let setup = Setup::new("openai", &params);
            let error = run(&mut bench, setup, CallKind::List, CallMode::Asynchronous)
                .await
                .expect_err("a text ssl_verify is a field error");
            assert_field(&error, "litellm_params.ssl_verify must be a boolean");
            assert!(bench.config.events().is_empty());
            assert!(bench.transport.source.async_calls().is_empty());
        }
        ErrorCase::ListApiBaseType => {
            let (mut bench, wire) = fake_bench(LIST, Reply::json(200, "{}"), RecordingConfig::new);
            let params = json_params(r#"{"api_base":1}"#);
            let setup = Setup::new("openai", &params);
            let error = run(&mut bench, setup, CallKind::List, CallMode::Asynchronous)
                .await
                .expect_err("a number api_base is a field error");
            assert_field(&error, "litellm_params.api_base must be a string");
            assert_eq!(bench.config.events(), ["validate"]);
            assert!(wire.requests().is_empty());
        }
    }
}

use std::time::Duration;

use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::{Serving, serve};
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::decisions::Json;
use rstest::rstest;
use tokio::runtime::Builder;

use super::support::{
    CallKind, EditPlan, Events, ExtensionPlan, Harness, Reply, Wire, expect_failure, headers,
    object, provider_error, proxied_client, proxied_client_with_read_timeout, run,
};
use crate::api::video_edit_transport::calls::Handled;
use crate::api::video_edit_transport::config::RequestFile;
use crate::api::video_edit_transport::edits::VideoEditHandlers as _;
use crate::api::video_edit_transport::error::VideoEditTransportError;
use crate::api::video_edit_transport::extensions::VideoExtensionHandlers as _;

#[rstest]
#[case::dod_7_extension_unconverted(dod_7_extension_unconverted)]
#[case::dod_7_edit_converted(dod_7_edit_converted)]
#[case::dod_8_prefetch_status(dod_8_prefetch_status)]
#[case::dod_8_prefetch_decode(dod_8_prefetch_decode)]
#[case::dod_8_status(dod_8_status)]
#[case::dod_8_response_transform(dod_8_response_transform)]
#[case::dod_8_send_failure(dod_8_send_failure)]
#[case::dod_8_build_failure(dod_8_build_failure)]
#[case::field_type(field_type)]
#[case::dod_9_sync_in_runtime(dod_9_sync_in_runtime)]
#[case::dod_1_async_hand_off_in_runtime(dod_1_async_hand_off_in_runtime)]
#[case::dod_8_edit_timeout(dod_8_edit_timeout)]
#[case::dod_8_extension_timeout(dod_8_extension_timeout)]
#[case::dod_8_timeout_without_a_caller_timeout(dod_8_timeout_without_a_caller_timeout)]
#[case::dod_8_async_timeout(dod_8_async_timeout)]
#[case::dod_8_extension_build_failure(dod_8_extension_build_failure)]
fn transport_error_behavior(#[case] check: fn()) {
    check();
}

fn dod_1_async_hand_off_in_runtime() {
    let events = Events::new();
    let wire = Wire::new(Vec::new(), &events);
    let mut harness = Harness::new(&events, wire.clone());
    harness.is_async = true;
    let transport = harness.transport();
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("the runtime builds");
    runtime.block_on(async {
        assert_pending(transport.video_edit(harness.edit_request(), &harness.config));
        assert_pending(transport.video_extension(harness.extension_request(), &harness.config));
    });
    assert!(harness.source.calls().is_empty());
    assert!(wire.requests().is_empty());
}

// The hand-off comes before the runtime guard, so _is_async still returns the async handler's future
fn assert_pending(handled: Result<Handled<'_>, VideoEditTransportError>) {
    let pending = handled.unwrap_or_else(|error| panic!("the sync handler failed: {error}"));
    assert!(
        matches!(pending, Handled::Pending(_)),
        "the sync handler ran instead of handing off"
    );
}

fn dod_7_extension_unconverted() {
    let events = Events::new();
    let wire = Wire::new(Vec::new(), &events);
    let mut harness = Harness::new(&events, wire.clone());
    let [transform_error, log_error] =
        harness.refusal_errors(CallKind::Extension, "the extension transform refused");
    let VideoEditTransportError::Config(source) = &transform_error else {
        panic!("the transform error was converted: {transform_error:?}");
    };
    assert_eq!(source.to_string(), "the extension transform refused");
    let VideoEditTransportError::Logging(PrePostError::CallbackFailed { message }) = &log_error
    else {
        panic!("the pre_call error was converted: {log_error:?}");
    };
    assert_eq!(message, "the pre_call refused");
    assert!(harness.config.errors().is_empty());
    assert!(harness.log.calls().is_empty());
    assert!(wire.requests().is_empty());
}

fn dod_7_edit_converted() {
    let events = Events::new();
    let wire = Wire::new(Vec::new(), &events);
    let mut harness = Harness::new(&events, wire.clone());
    let [transform_error, log_error] =
        harness.refusal_errors(CallKind::Edit, "the edit transform refused");
    assert_synthesized_error(transform_error, "the edit transform refused");
    assert_synthesized_error(log_error, "the pre_call refused");
    assert_eq!(
        harness.config.errors(),
        vec![
            ("the edit transform refused".to_owned(), 500, headers(&[])),
            ("the pre_call refused".to_owned(), 500, headers(&[])),
        ]
    );
    assert!(wire.requests().is_empty());
}

fn dod_8_prefetch_status() {
    let events = Events::new();
    let wire = Wire::new(
        vec![Reply::json(500, "prefetch failed").with_header("x-request-id", "abc")],
        &events,
    );
    let mut harness = Harness::new(&events, wire.clone());
    harness.config.prefetch = Some((
        "https://upstream.test/prefetch".to_owned(),
        object("{\"video_id\":\"video-77\"}"),
    ));
    let provider = provider_error(harness.call_error(CallKind::Edit));
    assert_eq!(provider.status_code, 500);
    assert!(!provider.status_code_is_synthesized);
    assert_eq!(provider.message, "prefetch failed");
    let length = "prefetch failed".len().to_string();
    let expected = headers(&[("content-length", length.as_str()), ("x-request-id", "abc")]);
    assert_eq!(
        harness.config.errors(),
        vec![("prefetch failed".to_owned(), 500, expected.clone())]
    );
    assert_eq!(
        provider.headers,
        Some(
            expected
                .iter()
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect()
        )
    );
    assert_eq!(wire.requests().len(), 1);
}

fn dod_8_prefetch_decode() {
    let events = Events::new();
    let wire = Wire::new(vec![Reply::json(200, "not json")], &events);
    let mut harness = Harness::new(&events, wire.clone());
    harness.config.prefetch = Some((
        "https://upstream.test/prefetch".to_owned(),
        object("{\"video_id\":\"video-77\"}"),
    ));
    let transport = harness.transport();
    let error = expect_failure(transport.video_edit(harness.edit_request(), &harness.config));
    assert!(matches!(error, VideoEditTransportError::PrefetchDecode(_)));
    assert!(harness.config.errors().is_empty());
    assert!(harness.config.edit_seen().is_none());
    assert_eq!(wire.requests().len(), 1);
    assert_eq!(events.recorded().join(","), "send");
}

fn dod_8_status() {
    let events = Events::new();
    let wire = Wire::new(
        vec![Reply::json(404, "gone"), Reply::json(404, "gone")],
        &events,
    );
    let harness = Harness::new(&events, wire.clone());
    for error in [
        harness.call_error(CallKind::Edit),
        harness.call_error(CallKind::Extension),
    ] {
        assert_reply_error(error, 404, "gone");
    }
    assert!(harness.config.edit_reply_seen().is_none());
    assert!(harness.config.extension_reply_seen().is_none());
    let expected = headers(&[("content-length", "4")]);
    assert_eq!(
        harness.config.errors(),
        vec![
            ("gone".to_owned(), 404, expected.clone()),
            ("gone".to_owned(), 404, expected),
        ]
    );
    assert_eq!(wire.requests().len(), 2);
}

// A response-backed error keeps the reply's status, headers and body text and is not synthesized
fn assert_reply_error(error: VideoEditTransportError, status: u16, body: &str) {
    let provider = provider_error(error);
    assert_eq!(provider.status_code, status);
    assert!(!provider.status_code_is_synthesized);
    assert_eq!(provider.message, body);
}

// An error with no response behind it is the provider's synthesized 500 whose message is the failure's
fn assert_synthesized_error(error: VideoEditTransportError, message: &str) {
    let provider = provider_error(error);
    assert_eq!(provider.status_code, 500);
    assert!(provider.status_code_is_synthesized);
    assert_eq!(provider.message, message);
}

fn dod_8_response_transform() {
    let body = "the reply body";
    let events = Events::new();
    let wire = Wire::new(
        vec![Reply::json(200, body), Reply::json(200, body)],
        &events,
    );
    let mut harness = Harness::new(&events, wire);
    harness.config.response_refusal = Some("cannot read the reply".to_owned());
    assert_reply_error(harness.call_error(CallKind::Edit), 200, body);
    assert_reply_error(harness.call_error(CallKind::Extension), 200, body);
    let length = body.len().to_string();
    let expected = headers(&[("content-length", length.as_str())]);
    assert_eq!(
        harness.config.errors(),
        vec![
            (body.to_owned(), 200, expected.clone()),
            (body.to_owned(), 200, expected),
        ]
    );
}

fn dod_8_send_failure() {
    let events = Events::new();
    let edit_wire = Wire::failing(TransportError::Closed, &events);
    let edit = Harness::new(&events, edit_wire.clone());
    let extension_wire = Wire::failing(TransportError::Closed, &events);
    let extension = Harness::new(&events, extension_wire.clone());
    for (kind, harness, wire) in [
        (CallKind::Edit, &edit, &edit_wire),
        (CallKind::Extension, &extension, &extension_wire),
    ] {
        assert_synthesized_error(
            harness.call_error(kind),
            &TransportError::Closed.to_string(),
        );
        assert_eq!(wire.requests().len(), 1);
    }
}

fn dod_8_build_failure() {
    let events = Events::new();
    let wire = Wire::new(Vec::new(), &events);
    let mut harness = Harness::new(&events, wire.clone());
    harness.config.edit = Some(EditPlan {
        url: "not a url".to_owned(),
        data: object("{}"),
        files: None,
        refusal: None,
    });
    let bad_url_transport = harness.transport();
    let bad_url_error =
        expect_failure(bad_url_transport.video_edit(harness.edit_request(), &harness.config));
    assert_synthesized_error(bad_url_error, "the request URL not a url is not a URL");
    harness.config.edit = Some(EditPlan {
        url: "https://upstream.test/edits".to_owned(),
        data: object("{\"nested\":{\"a\":1}}"),
        files: Some(vec![RequestFile {
            field: "video".to_owned(),
            filename: None,
            content: b"MP4".to_vec(),
            content_type: None,
        }]),
        refusal: None,
    });
    let form_transport = harness.transport();
    let form_error =
        expect_failure(form_transport.video_edit(harness.edit_request(), &harness.config));
    assert_synthesized_error(
        form_error,
        "the multipart form field nested must be a string, a number or a boolean",
    );
    assert!(wire.requests().is_empty());
}

fn dod_8_extension_build_failure() {
    let events = Events::new();
    let wire = Wire::new(Vec::new(), &events);
    let mut harness = Harness::new(&events, wire.clone());
    harness.config.extension = Some(ExtensionPlan {
        url: "not a url".to_owned(),
        data: object("{}"),
        refusal: None,
    });
    assert_synthesized_error(
        harness.call_error(CallKind::Extension),
        "the request URL not a url is not a URL",
    );
    assert!(wire.requests().is_empty());
}

fn dod_8_edit_timeout() {
    let (harness, _serving) = timeout_harness(proxied_client(), Some(Duration::from_millis(100)));
    assert_timeout_error(
        harness.call_error(CallKind::Edit),
        "litellm.Timeout: Connection timed out after 0.1 seconds.",
    );
}

fn dod_8_extension_timeout() {
    let (harness, _serving) = timeout_harness(proxied_client(), Some(Duration::from_millis(100)));
    assert_timeout_error(
        harness.call_error(CallKind::Extension),
        "litellm.Timeout: Connection timed out after 0.1 seconds.",
    );
}

// No caller timeout: the client's own read timeout ends each send, and both messages name none, as the handler's own timeout is None
fn dod_8_timeout_without_a_caller_timeout() {
    let (edit, _edit_serving) = timeout_harness(
        proxied_client_with_read_timeout(Duration::from_millis(100)),
        None,
    );
    assert_timeout_error(
        edit.call_error(CallKind::Edit),
        "litellm.Timeout: Connection timed out after None seconds.",
    );
    let (extension, _extension_serving) = timeout_harness(
        proxied_client_with_read_timeout(Duration::from_millis(100)),
        None,
    );
    let transport = extension.transport();
    let failed =
        run(transport.async_video_extension(extension.extension_request(), &extension.config))
            .expect_err("the timed-out call fails");
    assert_async_timeout(failed, "None");
}

fn dod_8_async_timeout() {
    for kind in [CallKind::Edit, CallKind::Extension] {
        let (harness, _serving) =
            timeout_harness(proxied_client(), Some(Duration::from_millis(100)));
        let transport = harness.transport();
        let failed = match kind {
            CallKind::Edit => {
                run(transport.async_video_edit(harness.edit_request(), &harness.config))
            }
            CallKind::Extension => {
                run(transport.async_video_extension(harness.extension_request(), &harness.config))
            }
        }
        .expect_err("the timed-out call fails");
        assert_async_timeout(failed, "0.1");
    }
}

// A call that times out: the fixture answers only after the client's 100ms deadline has run out
fn timeout_harness(
    client: ProxiedClient,
    timeout: Option<Duration>,
) -> (Harness<ProxiedClient>, Serving) {
    let serving = serve(reply("200 fixture", &[], "{}"), Duration::from_secs(2))
        .expect("the fixture server serves");
    let events = Events::new();
    let mut harness = Harness::new(&events, client);
    harness.timeout = timeout;
    let url = format!("http://{}/call", serving.address);
    harness.config.edit = Some(EditPlan {
        url: url.clone(),
        data: object("{\"prompt\":\"draw\"}"),
        files: None,
        refusal: None,
    });
    harness.config.extension = Some(ExtensionPlan {
        url,
        data: object("{\"seconds\":\"4\"}"),
        refusal: None,
    });
    (harness, serving)
}

// litellm.Timeout keeps its 408 with no response behind it, so the flag stays clear
fn assert_timeout_error(error: VideoEditTransportError, message: &str) {
    let provider = provider_error(error);
    assert_eq!(provider.status_code, 408);
    assert!(!provider.status_code_is_synthesized);
    assert_eq!(provider.message, message);
}

// AsyncHTTPHandler.post names the timeout it was given and the time the send took
fn assert_async_timeout(error: VideoEditTransportError, passed: &str) {
    let provider = provider_error(error);
    assert_eq!(provider.status_code, 408);
    assert!(!provider.status_code_is_synthesized);
    let prefix =
        format!("litellm.Timeout: Connection timed out. Timeout passed={passed}, time taken=");
    assert!(provider.message.starts_with(&prefix));
    assert!(provider.message.ends_with(" seconds"));
}

fn assert_field_type(error: VideoEditTransportError, field: &str, expected: &str) {
    match error {
        VideoEditTransportError::FieldType(seen) => {
            assert_eq!(seen.field, field);
            assert_eq!(seen.expected, expected);
        }
        other @ (VideoEditTransportError::Provider(_)
        | VideoEditTransportError::Config(_)
        | VideoEditTransportError::Logging(_)
        | VideoEditTransportError::Client(_)
        | VideoEditTransportError::PrefetchDecode(_)
        | VideoEditTransportError::SyncInRuntime { .. }
        | VideoEditTransportError::Runtime(_)) => {
            panic!("the wrong type travelled as {other:?}")
        }
    }
}

fn field_type() {
    let events = Events::new();
    let wire = Wire::new(Vec::new(), &events);
    let mut harness = Harness::new(&events, wire);
    harness
        .params
        .insert("api_base".to_owned(), Json::Bool(true));
    let base_transport = harness.transport();
    let base_error =
        expect_failure(base_transport.video_edit(harness.edit_request(), &harness.config));
    assert_field_type(base_error, "api_base", "a string");
    harness.params.insert(
        "api_base".to_owned(),
        Json::Text("https://upstream.test".to_owned()),
    );
    harness
        .params
        .insert("ssl_verify".to_owned(), Json::Text("yes".to_owned()));
    let flag_transport = harness.transport();
    let flag_error =
        expect_failure(flag_transport.video_edit(harness.edit_request(), &harness.config));
    assert_field_type(flag_error, "ssl_verify", "a boolean");
    assert!(harness.config.validated().is_none());
}

fn dod_9_sync_in_runtime() {
    let events = Events::new();
    let wire = Wire::new(Vec::new(), &events);
    let harness = Harness::new(&events, wire.clone());
    let transport = harness.transport();
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("the runtime builds");
    runtime.block_on(async {
        assert_sync_in_runtime(transport.video_edit(harness.edit_request(), &harness.config));
        assert_sync_in_runtime(
            transport.video_extension(harness.extension_request(), &harness.config),
        );
    });
    assert!(wire.requests().is_empty());
    assert!(harness.source.calls().is_empty());
    assert!(harness.config.errors().is_empty());
}

fn assert_sync_in_runtime(handled: Result<Handled<'_>, VideoEditTransportError>) {
    match expect_failure(handled) {
        VideoEditTransportError::SyncInRuntime { status_code } => assert_eq!(status_code, 500),
        other @ (VideoEditTransportError::Provider(_)
        | VideoEditTransportError::Config(_)
        | VideoEditTransportError::Logging(_)
        | VideoEditTransportError::Client(_)
        | VideoEditTransportError::PrefetchDecode(_)
        | VideoEditTransportError::FieldType(_)
        | VideoEditTransportError::Runtime(_)) => {
            panic!("the sync handler ran inside a runtime: {other:?}")
        }
    }
    let expected = VideoEditTransportError::SyncInRuntime { status_code: 500 };
    assert_eq!(
        expected.to_string(),
        "video sync handler cannot run inside a running runtime"
    );
}

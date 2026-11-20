use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::decisions::Json;
use rstest::rstest;
use tokio::runtime::Builder;

use super::support::{
    EditPlan, Events, ExtensionPlan, Harness, Reply, Wire, assert_nothing_sent, expect_failure,
    headers, object, provider_error, proxied_client, serve_once,
};
use crate::api::video_edit_transport::calls::Handled;
use crate::api::video_edit_transport::config::RequestFile;
use crate::api::video_edit_transport::error::VideoEditTransportError;

#[rstest]
#[case::dod_7_extension_transform_unconverted(dod_7_extension_transform_unconverted)]
#[case::dod_7_extension_pre_call_unconverted(dod_7_extension_pre_call_unconverted)]
#[case::dod_7_edit_transform_converted(dod_7_edit_transform_converted)]
#[case::dod_7_edit_pre_call_converted(dod_7_edit_pre_call_converted)]
#[case::dod_8_prefetch_status(dod_8_prefetch_status)]
#[case::dod_8_prefetch_decode(dod_8_prefetch_decode)]
#[case::dod_8_main_status(dod_8_main_status)]
#[case::dod_8_edit_response_transform(dod_8_edit_response_transform)]
#[case::dod_8_extension_response_transform(dod_8_extension_response_transform)]
#[case::dod_8_send_failure(dod_8_send_failure)]
#[case::dod_8_bad_request_url(dod_8_bad_request_url)]
#[case::dod_8_unencodable_form_field(dod_8_unencodable_form_field)]
#[case::field_type_api_base(field_type_api_base)]
#[case::field_type_ssl_verify(field_type_ssl_verify)]
#[case::dod_9_sync_in_runtime(dod_9_sync_in_runtime)]
fn transport_error_behavior(#[case] check: fn()) {
    check();
}

fn dod_7_extension_transform_unconverted() {
    let events = Events::new();
    let server = serve_once("200 fixture", &[], "{}");
    let mut harness = Harness::new(&events, proxied_client());
    harness.config.extension = Some(ExtensionPlan {
        url: format!("http://{}/extensions", server.address),
        data: object("{}"),
        refusal: Some("the extension transform refused".to_owned()),
    });
    let transport = harness.transport();
    let error =
        expect_failure(transport.video_extension(harness.extension_request(), &harness.config));
    let message = match error {
        VideoEditTransportError::Config(source) => source.to_string(),
        other => panic!("the transform error was converted: {other:?}"),
    };
    assert_eq!(message, "the extension transform refused");
    assert!(harness.config.errors().is_empty());
    assert!(harness.log.calls().is_empty());
    assert_nothing_sent(server);
}

fn dod_7_extension_pre_call_unconverted() {
    let events = Events::new();
    let server = serve_once("200 fixture", &[], "{}");
    let mut harness = Harness::new(&events, proxied_client());
    harness.log.refusal = Some("the pre_call refused".to_owned());
    harness.config.extension = Some(ExtensionPlan {
        url: format!("http://{}/extensions", server.address),
        data: object("{}"),
        refusal: None,
    });
    let transport = harness.transport();
    let error =
        expect_failure(transport.video_extension(harness.extension_request(), &harness.config));
    let message = match error {
        VideoEditTransportError::Logging(PrePostError::CallbackFailed { message }) => message,
        other => panic!("the pre_call error was converted: {other:?}"),
    };
    assert_eq!(message, "the pre_call refused");
    assert!(harness.config.errors().is_empty());
    assert_nothing_sent(server);
}

fn dod_7_edit_transform_converted() {
    let events = Events::new();
    let server = serve_once("200 fixture", &[], "{}");
    let mut harness = Harness::new(&events, proxied_client());
    harness.config.edit = Some(EditPlan {
        url: format!("http://{}/edits", server.address),
        data: object("{}"),
        files: None,
        refusal: Some("the edit transform refused".to_owned()),
    });
    let transport = harness.transport();
    let error = expect_failure(transport.video_edit(harness.edit_request(), &harness.config));
    let provider = provider_error(error);
    assert_eq!(provider.status_code, 500);
    assert!(provider.status_code_is_synthesized);
    assert_eq!(provider.message, "the edit transform refused");
    assert_eq!(
        harness.config.errors(),
        vec![("the edit transform refused".to_owned(), 500, headers(&[]))]
    );
    assert_nothing_sent(server);
}

fn dod_7_edit_pre_call_converted() {
    let events = Events::new();
    let wire = Wire::new(Vec::new(), &events);
    let mut harness = Harness::new(&events, wire.clone());
    harness.log.refusal = Some("the pre_call refused".to_owned());
    let transport = harness.transport();
    let error = expect_failure(transport.video_edit(harness.edit_request(), &harness.config));
    let provider = provider_error(error);
    assert_eq!(provider.status_code, 500);
    assert!(provider.status_code_is_synthesized);
    assert_eq!(provider.message, "the pre_call refused");
    assert_eq!(
        harness.config.errors(),
        vec![("the pre_call refused".to_owned(), 500, headers(&[]))]
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
    let transport = harness.transport();
    let error = expect_failure(transport.video_edit(harness.edit_request(), &harness.config));
    let provider = provider_error(error);
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

fn dod_8_main_status() {
    let events = Events::new();
    let wire = Wire::new(vec![Reply::json(404, "gone")], &events);
    let harness = Harness::new(&events, wire.clone());
    let transport = harness.transport();
    let error = expect_failure(transport.video_edit(harness.edit_request(), &harness.config));
    let provider = provider_error(error);
    assert_eq!(provider.status_code, 404);
    assert!(!provider.status_code_is_synthesized);
    assert_eq!(provider.message, "gone");
    assert_eq!(
        harness.config.errors(),
        vec![("gone".to_owned(), 404, headers(&[("content-length", "4")]))]
    );
    assert!(harness.config.edit_reply_seen().is_none());
    assert_eq!(wire.requests().len(), 1);
}

fn dod_8_edit_response_transform() {
    let body = "the reply body";
    let events = Events::new();
    let wire = Wire::new(vec![Reply::json(200, body)], &events);
    let mut harness = Harness::new(&events, wire.clone());
    harness.config.response_refusal = Some("cannot read the reply".to_owned());
    let transport = harness.transport();
    let error = expect_failure(transport.video_edit(harness.edit_request(), &harness.config));
    let provider = provider_error(error);
    assert_eq!(provider.status_code, 200);
    assert!(!provider.status_code_is_synthesized);
    assert_eq!(provider.message, body);
    let length = body.len().to_string();
    assert_eq!(
        harness.config.errors(),
        vec![(
            body.to_owned(),
            200,
            headers(&[("content-length", length.as_str())])
        )]
    );
}

fn dod_8_extension_response_transform() {
    let body = "the extension body";
    let events = Events::new();
    let wire = Wire::new(vec![Reply::json(200, body)], &events);
    let mut harness = Harness::new(&events, wire.clone());
    harness.config.response_refusal = Some("cannot read the reply".to_owned());
    let transport = harness.transport();
    let error =
        expect_failure(transport.video_extension(harness.extension_request(), &harness.config));
    let provider = provider_error(error);
    assert_eq!(provider.status_code, 200);
    assert!(!provider.status_code_is_synthesized);
    assert_eq!(provider.message, body);
    let length = body.len().to_string();
    assert_eq!(
        harness.config.errors(),
        vec![(
            body.to_owned(),
            200,
            headers(&[("content-length", length.as_str())])
        )]
    );
}

fn dod_8_send_failure() {
    let events = Events::new();
    let wire = Wire::failing(TransportError::Closed, &events);
    let harness = Harness::new(&events, wire.clone());
    let transport = harness.transport();
    let error = expect_failure(transport.video_edit(harness.edit_request(), &harness.config));
    let provider = provider_error(error);
    assert_eq!(provider.status_code, 500);
    assert!(provider.status_code_is_synthesized);
    assert_eq!(provider.message, TransportError::Closed.to_string());
    assert_eq!(wire.requests().len(), 1);
}

fn dod_8_bad_request_url() {
    let events = Events::new();
    let wire = Wire::new(Vec::new(), &events);
    let mut harness = Harness::new(&events, wire.clone());
    harness.config.edit = Some(EditPlan {
        url: "not a url".to_owned(),
        data: object("{}"),
        files: None,
        refusal: None,
    });
    let transport = harness.transport();
    let error = expect_failure(transport.video_edit(harness.edit_request(), &harness.config));
    let provider = provider_error(error);
    assert_eq!(provider.status_code, 500);
    assert!(provider.status_code_is_synthesized);
    assert_eq!(provider.message, "the request URL not a url is not a URL");
    assert!(wire.requests().is_empty());
}

fn dod_8_unencodable_form_field() {
    let events = Events::new();
    let wire = Wire::new(Vec::new(), &events);
    let mut harness = Harness::new(&events, wire.clone());
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
    let transport = harness.transport();
    let error = expect_failure(transport.video_edit(harness.edit_request(), &harness.config));
    let provider = provider_error(error);
    assert_eq!(provider.status_code, 500);
    assert!(provider.status_code_is_synthesized);
    assert_eq!(
        provider.message,
        "the multipart form field nested must be a string, a number or a boolean"
    );
    assert!(wire.requests().is_empty());
}

fn field_type_api_base() {
    let events = Events::new();
    let wire = Wire::new(Vec::new(), &events);
    let mut harness = Harness::new(&events, wire.clone());
    harness
        .params
        .insert("api_base".to_owned(), Json::Bool(true));
    let transport = harness.transport();
    let error = expect_failure(transport.video_edit(harness.edit_request(), &harness.config));
    match error {
        VideoEditTransportError::FieldType(field) => {
            assert_eq!(field.field, "api_base");
            assert_eq!(field.expected, "a string");
        }
        other => panic!("the wrong type travelled as {other:?}"),
    }
    assert!(harness.config.validated().is_none());
}

fn field_type_ssl_verify() {
    let events = Events::new();
    let wire = Wire::new(Vec::new(), &events);
    let mut harness = Harness::new(&events, wire.clone());
    harness
        .params
        .insert("ssl_verify".to_owned(), Json::Text("yes".to_owned()));
    let transport = harness.transport();
    let error = expect_failure(transport.video_edit(harness.edit_request(), &harness.config));
    match error {
        VideoEditTransportError::FieldType(field) => {
            assert_eq!(field.field, "ssl_verify");
            assert_eq!(field.expected, "a boolean");
        }
        other => panic!("the wrong type travelled as {other:?}"),
    }
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
        other => panic!("the sync handler ran inside a runtime: {other:?}"),
    }
    let expected = VideoEditTransportError::SyncInRuntime { status_code: 500 };
    assert_eq!(
        expected.to_string(),
        "video sync handler cannot run inside a running runtime"
    );
}

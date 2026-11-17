#![cfg(test)]

mod support;

use std::collections::BTreeMap;

use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::openai_video_identifiers::ids::decode_video_id_with_provider;
use betterlite_openai::videos::reply::VideoObjectType;
use http::header::HeaderMap;
use rstest::rstest;
use tokio::net::TcpListener;
use tokio::runtime::Builder;
use url::Url;

use super::content::{content_response, extract_video_url};
use super::error::VideoResultsError;
use super::status::{
    StatusCall, StatusRequest, result_request, status_response, status_video_object,
};
use super::text::{error_text, result_error, terminal_result_error};
use super::transport::{
    ClientKind, SuppliedTransport, VideoClients, VideoTransport as _, header_map,
};
use super::unsupported::{ALL, UnsupportedOperation};
use crate::api::fal::video_prepare::VideoHeaders;
use crate::api::fal::video_prepare::decode::decode_fal_video_reply;
use crate::api::fal::video_prepare::error::FalVideoError;
use crate::api::fal::video_results::tests::support::{
    FakeTransport, MODEL_PATH, SHORT_READ_LIMIT, SMALL_BODY_LIMIT, STATUS_URL, Step, answered,
    block_on_status, content_reply, decoded, headers, hold_open, message, mock_failure,
    outbound_client, run_content, run_status, serve_artifact, status_reply, status_request,
};

#[test]
fn a_completed_job_probe_error_fails_the_video() {
    let transport = FakeTransport::new(answered(
        400,
        r#"{"detail": [{"msg": "prompt is too long", "loc": ["prompt"]}]}"#,
    ));
    let reply = status_reply(r#"{"status": "COMPLETED", "request_id": "req-1"}"#);
    let call = StatusCall {
        request: status_request(),
        reply: &reply,
        provider: None,
    };
    let video = run_status(&transport, ClientKind::Sync, &call).expect("a status video");
    assert_eq!(video.status, "failed");
    assert_eq!(video.created_at, Some(0));
    assert_eq!(video.model.as_deref(), Some(MODEL_PATH));
    assert_eq!(
        message(&video).as_deref(),
        Some("prompt: prompt is too long")
    );
    assert_eq!(
        decoded(&video),
        (
            Some("fal_ai".to_owned()),
            Some(MODEL_PATH.to_owned()),
            "req-1".to_owned()
        )
    );
    let error = video.error.expect("a failed video carries an error");
    assert_eq!(error["code"], "fal_error");
}

#[rstest]
#[case::server_error(Step::Answer(500, b"boom".to_vec()))]
#[case::rate_limited(Step::Answer(429, b"rate limited".to_vec()))]
#[case::unavailable(Step::Answer(503, b"".to_vec()))]
#[case::unsupported_scheme(Step::Fail(TransportError::RedirectScheme {
    scheme: Box::from("ftp")
}))]
#[case::mock_failure(mock_failure())]
fn a_transient_probe_failure_keeps_the_completed_status(#[case] step: Step) {
    let transport = FakeTransport::new(vec![step]);
    let reply = status_reply(r#"{"status": "COMPLETED", "request_id": "req-1"}"#);
    let call = StatusCall {
        request: status_request(),
        reply: &reply,
        provider: None,
    };
    let video = run_status(&transport, ClientKind::Sync, &call).expect("a status video");
    assert_eq!(video.status, "completed");
    assert!(video.error.is_none());
    assert_eq!(transport.recorded().len(), 1);
}

#[rstest]
#[case::redirect_loop(TransportError::TooManyRedirects { limit: 10 })]
#[case::shutting_down(TransportError::Closed)]
#[case::capped_answer(TransportError::BodyTooLarge { limit: 16 })]
#[case::insecure_redirect(TransportError::InsecureRedirect)]
fn a_non_transient_probe_failure_fails_the_video(#[case] source: TransportError) {
    let transport = FakeTransport::new(vec![Step::Fail(source)]);
    let reply = status_reply(r#"{"status": "COMPLETED", "request_id": "req-1"}"#);
    let call = StatusCall {
        request: status_request(),
        reply: &reply,
        provider: None,
    };
    let error = run_status(&transport, ClientKind::Sync, &call).expect_err("a failed probe");
    assert!(matches!(error, VideoResultsError::ProbeTransport { .. }));
    assert_eq!(transport.recorded().len(), 1);
}

#[tokio::test]
async fn a_refused_probe_connection_keeps_the_completed_status() {
    let address = {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("the listener");
        listener.local_addr().expect("the address")
    };
    let client = outbound_client(SHORT_READ_LIMIT);
    let clients = VideoClients {
        sync: &client,
        asynchronous: &client,
    };
    let probe_url = Url::parse(&format!("http://{address}/probe")).expect("the probe url");
    let refused = client
        .get(probe_url, HeaderMap::new())
        .await
        .expect_err("nothing listens on the address");
    assert!(matches!(refused, TransportError::Connect(_)));
    let reply = status_reply(r#"{"status": "COMPLETED", "request_id": "req-1"}"#);
    let call = StatusCall {
        request: StatusRequest {
            url: Url::parse(&format!(
                "http://{address}/fal-ai/kling-video/requests/req-1/status"
            ))
            .expect("the status url"),
            headers: VideoHeaders::new(),
        },
        reply: &reply,
        provider: None,
    };
    let video = status_response(&clients, ClientKind::Sync, None, &call)
        .await
        .expect("a status video");
    assert_eq!(video.status, "completed");
    assert!(video.error.is_none());
}

#[tokio::test(start_paused = true)]
async fn a_stalled_probe_connection_fails_with_the_read_limit() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("the listener");
    let address = listener.local_addr().expect("the address");
    let client = outbound_client(SHORT_READ_LIMIT);
    let probe_url = Url::parse(&format!("http://{address}/probe")).expect("the probe url");
    let (stalled, ()) = tokio::join!(client.get(probe_url, HeaderMap::new()), hold_open(listener));
    assert!(matches!(
        stalled.expect_err("the stall ends the request"),
        TransportError::Deadline(_)
    ));
}

#[tokio::test(start_paused = true)]
async fn a_probe_past_the_read_limit_keeps_the_completed_status() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("the listener");
    let address = listener.local_addr().expect("the address");
    let client = outbound_client(SHORT_READ_LIMIT);
    let clients = VideoClients {
        sync: &client,
        asynchronous: &client,
    };
    let reply = status_reply(r#"{"status": "COMPLETED", "request_id": "req-1"}"#);
    let call = StatusCall {
        request: StatusRequest {
            url: Url::parse(&format!(
                "http://{address}/fal-ai/kling-video/requests/req-1/status"
            ))
            .expect("the status url"),
            headers: VideoHeaders::new(),
        },
        reply: &reply,
        provider: None,
    };
    let (result, ()) = tokio::join!(
        status_response(&clients, ClientKind::Sync, None, &call),
        hold_open(listener)
    );
    let video = result.expect("a status video");
    assert_eq!(video.status, "completed");
    assert!(video.error.is_none());
}

#[test]
fn the_result_probe_repeats_the_request_without_transport_headers() {
    let transport = FakeTransport::new(answered(400, "plain failure"));
    let reply = status_reply(r#"{"status": "COMPLETED"}"#);
    let call = StatusCall {
        request: status_request(),
        reply: &reply,
        provider: None,
    };
    let video = run_status(&transport, ClientKind::Sync, &call).expect("a status video");
    assert_eq!(message(&video).as_deref(), Some("plain failure"));
    let probe = transport.recorded();
    assert_eq!(probe.len(), 1);
    assert_eq!(
        probe[0].0,
        "https://queue.fal.run/fal-ai/kling-video/requests/req-1"
    );
    assert_eq!(
        probe[0].1,
        BTreeMap::from([
            ("authorization".to_owned(), "Bearer fal-key".to_owned()),
            ("x-trace".to_owned(), "t-1".to_owned()),
        ])
    );
}

#[rstest]
#[case::in_progress(r#"{"status": "IN_PROGRESS"}"#)]
#[case::in_queue(r#"{"status": "IN_QUEUE"}"#)]
#[case::unknown(r#"{"status": "QUEUED"}"#)]
#[case::absent("{}")]
#[case::not_text(r#"{"status": 7}"#)]
#[case::empty(r#"{"status": ""}"#)]
fn only_a_completed_status_is_probed(#[case] body: &str) {
    let transport = FakeTransport::idle();
    let reply = status_reply(body);
    let call = StatusCall {
        request: status_request(),
        reply: &reply,
        provider: None,
    };
    let video = run_status(&transport, ClientKind::Sync, &call).expect("a status video");
    assert!(transport.recorded().is_empty());
    assert_ne!(video.status, "failed");
}

#[rstest]
#[case::text(
    r#"{"status":"IN_PROGRESS","detail":"x is broken"}"#,
    Some("x is broken")
)]
#[case::empty_text_shadows_error(r#"{"status":"IN_PROGRESS","detail":"","error":"boom"}"#, None)]
#[case::empty_list_falls_through(
    r#"{"status":"IN_PROGRESS","detail":[],"error":"boom"}"#,
    Some("boom")
)]
#[case::object_detail_falls_through(
    r#"{"status":"IN_PROGRESS","detail":{},"error":"boom"}"#,
    Some("boom")
)]
#[case::located_item(
    r#"{"status":"IN_PROGRESS","detail":[{"msg":"a","loc":"body"}],"error":"boom"}"#,
    Some("body: a")
)]
#[case::joined_locations(
    r#"{"status":"IN_PROGRESS","detail":[{"msg":"a","loc":["x",1,"y"]},{"msg":"b"}]}"#,
    Some("x.y: a; b")
)]
#[case::empty_locations(
    r#"{"status":"IN_PROGRESS","detail":[{"msg":"c","loc":""},{"msg":"d","loc":[]}]}"#,
    Some("c; d")
)]
#[case::single_location_part(
    r#"{"status":"IN_PROGRESS","detail":[{"msg":"e","loc":["x",1]}]}"#,
    Some("x: e")
)]
#[case::message_not_text(
    r#"{"status":"IN_PROGRESS","detail":[{"msg":7,"loc":"z"}],"error":"boom"}"#,
    Some("boom")
)]
#[case::error_not_text(r#"{"status":"IN_PROGRESS","error":7}"#, None)]
#[case::empty_error(r#"{"status":"IN_PROGRESS","error":""}"#, None)]
fn detail_text_decides_before_the_error_field(#[case] body: &str, #[case] expected: Option<&str>) {
    let transport = FakeTransport::idle();
    let reply = status_reply(body);
    let call = StatusCall {
        request: status_request(),
        reply: &reply,
        provider: None,
    };
    let video = run_status(&transport, ClientKind::Sync, &call).expect("a status video");
    assert_eq!(message(&video).as_deref(), expected);
    assert_eq!(video.status == "failed", expected.is_some());
}

#[rstest]
#[case::in_queue(r#"{"status": "IN_QUEUE"}"#, "queued")]
#[case::in_progress(r#"{"status": "IN_PROGRESS"}"#, "in_progress")]
#[case::completed(r#"{"status": "COMPLETED"}"#, "completed")]
#[case::unknown(r#"{"status": "STARTED"}"#, "queued")]
#[case::empty(r#"{"status": ""}"#, "queued")]
#[case::null(r#"{"status": null}"#, "queued")]
#[case::absent("{}", "queued")]
fn the_status_maps_the_provider_states(#[case] body: &str, #[case] expected: &str) {
    let transport = FakeTransport::new(answered(200, "{}"));
    let reply = status_reply(body);
    let call = StatusCall {
        request: status_request(),
        reply: &reply,
        provider: None,
    };
    let video = run_status(&transport, ClientKind::Sync, &call).expect("a status video");
    assert_eq!(video.status, expected);
}

#[rstest]
#[case::absent(r#"{"status": "IN_QUEUE"}"#, "req-1")]
#[case::empty(r#"{"status": "IN_QUEUE", "request_id": ""}"#, "req-1")]
#[case::not_text(r#"{"status": "IN_QUEUE", "request_id": 7}"#, "req-1")]
#[case::from_body(r#"{"status": "IN_QUEUE", "request_id": "body-req"}"#, "body-req")]
fn the_request_id_comes_from_the_body_then_the_request_url(
    #[case] body: &str,
    #[case] expected: &str,
) {
    let transport = FakeTransport::idle();
    let reply = status_reply(body);
    let call = StatusCall {
        request: status_request(),
        reply: &reply,
        provider: None,
    };
    let video = run_status(&transport, ClientKind::Sync, &call).expect("a status video");
    assert_eq!(decoded(&video).2, expected);
    assert_eq!(decoded(&video).1.as_deref(), Some(MODEL_PATH));
}

#[test]
fn a_percent_encoded_request_segment_splits_like_httpx() {
    let transport = FakeTransport::idle();
    let reply = status_reply(r#"{"status": "IN_QUEUE"}"#);
    let call = StatusCall {
        request: StatusRequest {
            url: Url::parse("https://queue.fal.run/fal-ai/kling-video/requests/req%2Fslash/status")
                .expect("the status url parses"),
            headers: VideoHeaders::new(),
        },
        reply: &reply,
        provider: None,
    };
    let video = run_status(&transport, ClientKind::Sync, &call).expect("a status video");
    assert_eq!(decoded(&video).2, "req");
    assert_eq!(decoded(&video).1.as_deref(), Some(MODEL_PATH));
}

#[test]
fn a_url_without_a_requests_segment_leaves_the_identity_unwrapped() {
    let transport = FakeTransport::idle();
    let reply = status_reply(r#"{"status": "IN_QUEUE", "request_id": "req-9"}"#);
    let call = StatusCall {
        request: StatusRequest {
            url: Url::parse("https://queue.fal.run/fal-ai/kling-video/status")
                .expect("the status url parses"),
            headers: VideoHeaders::new(),
        },
        reply: &reply,
        provider: None,
    };
    let video = run_status(&transport, ClientKind::Sync, &call).expect("a status video");
    assert_eq!(video.model, None);
    let id = decode_video_id_with_provider(&video.id);
    assert_eq!(id.custom_llm_provider.as_deref(), Some("fal_ai"));
    assert_eq!(id.model_id.as_deref(), Some(""));
    assert_eq!(id.video_id, "req-9");
}

#[rstest]
#[case::default(None, "fal_ai")]
#[case::empty(Some(""), "fal_ai")]
#[case::custom(Some("fal_ai_custom"), "fal_ai_custom")]
fn the_provider_wraps_the_video_id(#[case] provider: Option<&str>, #[case] expected: &str) {
    let transport = FakeTransport::idle();
    let reply = status_reply(r#"{"status": "IN_QUEUE"}"#);
    let call = StatusCall {
        request: status_request(),
        reply: &reply,
        provider,
    };
    let video = run_status(&transport, ClientKind::Sync, &call).expect("a status video");
    assert_eq!(decoded(&video).0.as_deref(), Some(expected));
}

#[test]
fn the_probe_url_only_loses_a_trailing_status_suffix() {
    let reply = decode_fal_video_reply(
        200,
        &VideoHeaders::new(),
        br#"{"status": "COMPLETED", "response_url": "https://cdn.example/elsewhere"}"#,
    )
    .expect("the reply decodes");
    let request = StatusRequest {
        url: Url::parse("https://queue.fal.run/fal-ai/kling-video/requests/req-1")
            .expect("the job url parses"),
        headers: VideoHeaders::new(),
    };
    let (url, headers) = result_request(&reply, &request).expect("a completed job probes");
    assert_eq!(
        url,
        "https://queue.fal.run/fal-ai/kling-video/requests/req-1"
    );
    assert!(headers.is_empty());

    let queried = StatusRequest {
        url: Url::parse(
            "https://queue.fal.run/fal-ai/kling-video/requests/req-1/status?x=1&x=2&y=",
        )
        .expect("the queried status url parses"),
        headers: VideoHeaders::new(),
    };
    let (queried_url, _) = result_request(&reply, &queried).expect("a completed job probes");
    assert_eq!(queried_url, queried.url.as_str());
}

#[rstest]
#[case::body_text(400, "plain failure", Some("plain failure"))]
#[case::empty_body(400, "", Some("fal.ai returned HTTP 400"))]
#[case::blank_body(418, " ", Some(" "))]
#[case::redirect(302, "moved", Some("moved"))]
#[case::rate_limited(429, "slow down", None)]
#[case::server_error(500, "boom", None)]
#[case::success(200, "{}", None)]
#[case::empty_success(204, "", None)]
#[case::empty_detail_body_text(
    400,
    r#"{"detail": "", "error": "boom"}"#,
    Some(r#"{"detail": "", "error": "boom"}"#)
)]
fn a_failed_probe_names_its_body_or_status(
    #[case] status: u16,
    #[case] body: &str,
    #[case] expected: Option<&str>,
) {
    assert_eq!(
        terminal_result_error(status, body.as_bytes()).as_deref(),
        expected
    );
}

#[test]
fn a_probe_body_that_is_not_utf8_names_only_the_status() {
    assert_eq!(
        terminal_result_error(400, &[0xff, 0xfe]).as_deref(),
        Some("fal.ai returned HTTP 400")
    );
}

#[rstest]
#[case::detail(
    r#"{"detail": [{"msg": "upstream exploded", "loc": "job"}]}"#,
    "job: upstream exploded"
)]
#[case::body_text("not json", "not json")]
fn a_failed_result_status_raises_the_provider_error(#[case] body: &str, #[case] expected: &str) {
    let transport = FakeTransport::idle();
    let reply = content_reply(503, headers(&[("X-Request-Id", "r-1")]), body.as_bytes());
    let error = run_content(&transport, ClientKind::Sync, &reply).expect_err("a failed result");
    let VideoResultsError::Upstream(exception) = error else {
        panic!("a provider error");
    };
    assert_eq!(exception.status_code, 503);
    assert_eq!(exception.headers, headers(&[("X-Request-Id", "r-1")]));
    assert_eq!(exception.message, expected);
    assert!(transport.recorded().is_empty());
}

#[test]
fn an_unsuccessful_result_body_that_is_not_utf8_names_only_the_status() {
    let transport = FakeTransport::idle();
    let reply = content_reply(400, VideoHeaders::new(), &[0xff, 0xfe]);
    let error = run_content(&transport, ClientKind::Sync, &reply).expect_err("a failed result");
    let VideoResultsError::Upstream(exception) = error else {
        panic!("a provider error");
    };
    assert_eq!(exception.message, "fal.ai returned HTTP 400");
}

#[test]
fn content_downloads_the_artifact_without_request_headers() {
    let transport = FakeTransport::new(vec![Step::Answer(200, b"video-bytes".to_vec())]);
    let reply = content_reply(
        200,
        headers(&[("Authorization", "Bearer fal-key")]),
        br#"{"video": {"url": "https://cdn.fal.run/out/video.mp4?token=abc"}}"#,
    );
    let body = run_content(&transport, ClientKind::Sync, &reply).expect("the artifact bytes");
    assert_eq!(body, b"video-bytes".to_vec());
    let artifact = transport.recorded();
    assert_eq!(artifact.len(), 1);
    assert_eq!(artifact[0].0, "https://cdn.fal.run/out/video.mp4?token=abc");
    assert!(artifact[0].1.is_empty());
}

#[test]
fn artifact_bytes_come_back_unchanged() {
    let bytes = vec![0x00, 0xff, 0x10, 0x80];
    let transport = FakeTransport::new(vec![Step::Answer(200, bytes.clone())]);
    let reply = content_reply(
        200,
        VideoHeaders::new(),
        br#"{"video": {"url": "https://cdn.fal.run/out/video.bin"}}"#,
    );
    let body =
        run_content(&transport, ClientKind::Asynchronous, &reply).expect("the artifact bytes");
    assert_eq!(body, bytes);
}

#[test]
fn an_unsuccessful_artifact_status_is_an_error() {
    let transport = FakeTransport::new(vec![Step::Answer(404, b"gone".to_vec())]);
    let reply = content_reply(
        200,
        VideoHeaders::new(),
        br#"{"video": {"url": "https://cdn.fal.run/out/video.mp4"}}"#,
    );
    let error = run_content(&transport, ClientKind::Sync, &reply).expect_err("a failed download");
    assert!(matches!(
        error,
        VideoResultsError::ArtifactStatus { status: 404 }
    ));
    assert_eq!(transport.recorded().len(), 1);
}

#[test]
fn an_artifact_transport_failure_is_an_error() {
    let transport = FakeTransport::new(vec![Step::Fail(TransportError::RedirectScheme {
        scheme: Box::from("ftp"),
    })]);
    let reply = content_reply(
        200,
        VideoHeaders::new(),
        br#"{"video": {"url": "https://cdn.fal.run/out/video.mp4"}}"#,
    );
    let error = run_content(&transport, ClientKind::Sync, &reply).expect_err("a failed download");
    assert!(matches!(error, VideoResultsError::ArtifactTransport(_)));
}

#[tokio::test]
async fn an_artifact_over_the_transport_body_limit_is_a_transport_error() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("the listener");
    let address = listener.local_addr().expect("the address");
    let client = outbound_client(SMALL_BODY_LIMIT);
    let clients = VideoClients {
        sync: &client,
        asynchronous: &client,
    };
    let body = format!(r#"{{"video": {{"url": "http://{address}/out.mp4"}}}}"#);
    let reply = content_reply(200, VideoHeaders::new(), body.as_bytes());
    let (result, ()) = tokio::join!(
        content_response(&clients, ClientKind::Sync, &reply),
        serve_artifact(
            listener,
            b"HTTP/1.1 200 OK\r\ncontent-length: 5\r\nconnection: close\r\n\r\n12345"
        )
    );
    let error = result.expect_err("the artifact is over the limit");
    assert!(matches!(
        error,
        VideoResultsError::ArtifactTransport(TransportError::BodyTooLarge { limit: 4 })
    ));
}

#[rstest]
#[case::empty_object("{}", "fal.ai video result did not include a video URL")]
#[case::empty_video(r#"{"video": {}}"#, "fal.ai video result did not include a video URL")]
#[case::empty_url(
    r#"{"video": {"url": ""}}"#,
    "fal.ai video result did not include a video URL"
)]
#[case::url_not_text(
    r#"{"video": {"url": 7}}"#,
    "fal.ai video result did not include a video URL"
)]
#[case::video_not_object(r#"{"video": []}"#, "fal.ai video result did not include a video URL")]
#[case::detail_text(
    r#"{"detail": "broken"}"#,
    "fal.ai video result did not include a video URL: broken"
)]
#[case::detail_items(
    r#"{"detail": [{"msg": "a", "loc": ["b", 1]}]}"#,
    "fal.ai video result did not include a video URL: b: a"
)]
#[case::empty_detail_shadows_error(
    r#"{"detail": "", "error": "boom"}"#,
    "fal.ai video result did not include a video URL"
)]
#[case::error_text(
    r#"{"error": "boom"}"#,
    "fal.ai video result did not include a video URL: boom"
)]
fn a_missing_video_url_reports_its_detail(#[case] body: &str, #[case] expected: &str) {
    let transport = FakeTransport::idle();
    let reply = content_reply(200, VideoHeaders::new(), body.as_bytes());
    let error = run_content(&transport, ClientKind::Sync, &reply).expect_err("a missing url");
    assert_eq!(error.to_string(), expected);
    assert!(transport.recorded().is_empty());
}

#[test]
fn the_extracted_video_url_comes_from_the_reply_only() {
    let reply = decode_fal_video_reply(
        200,
        &VideoHeaders::new(),
        br#"{"video": {"url": "https://cdn.fal.run/a.mp4"}, "detail": "ignored"}"#,
    )
    .expect("the reply decodes");
    assert_eq!(
        extract_video_url(&reply).expect("a video url"),
        "https://cdn.fal.run/a.mp4"
    );
}

#[test]
fn a_supplied_transport_of_the_matching_kind_answers_the_probe() {
    let default = FakeTransport::new(answered(400, "default failure"));
    let supplied = FakeTransport::new(answered(400, "supplied failure"));
    let clients = VideoClients {
        sync: &default,
        asynchronous: &default,
    };
    let reply = status_reply(r#"{"status": "COMPLETED"}"#);
    let call = StatusCall {
        request: status_request(),
        reply: &reply,
        provider: None,
    };
    let supplied_transport = SuppliedTransport {
        kind: ClientKind::Sync,
        transport: &supplied,
    };
    let video = block_on_status(&clients, ClientKind::Sync, Some(&supplied_transport), &call)
        .expect("a status video");
    assert_eq!(message(&video).as_deref(), Some("supplied failure"));
    assert_eq!(supplied.recorded().len(), 1);
    assert!(default.recorded().is_empty());
}

#[test]
fn a_supplied_transport_of_the_other_kind_is_ignored() {
    let default = FakeTransport::new(answered(400, "default failure"));
    let supplied = FakeTransport::new(answered(400, "supplied failure"));
    let clients = VideoClients {
        sync: &default,
        asynchronous: &default,
    };
    let reply = status_reply(r#"{"status": "COMPLETED"}"#);
    let call = StatusCall {
        request: status_request(),
        reply: &reply,
        provider: None,
    };
    let supplied_transport = SuppliedTransport {
        kind: ClientKind::Asynchronous,
        transport: &supplied,
    };
    let video = block_on_status(&clients, ClientKind::Sync, Some(&supplied_transport), &call)
        .expect("a status video");
    assert_eq!(message(&video).as_deref(), Some("default failure"));
    assert_eq!(default.recorded().len(), 1);
    assert!(supplied.recorded().is_empty());
}

#[test]
fn the_sync_and_async_paths_share_the_status_decisions() {
    let sync_transport = FakeTransport::new(answered(400, "terminal failure"));
    let async_transport = FakeTransport::new(answered(400, "terminal failure"));
    let clients = VideoClients {
        sync: &sync_transport,
        asynchronous: &async_transport,
    };
    let reply = status_reply(r#"{"status": "COMPLETED", "request_id": "req-1"}"#);
    let call = StatusCall {
        request: status_request(),
        reply: &reply,
        provider: None,
    };
    let sync_video =
        block_on_status(&clients, ClientKind::Sync, None, &call).expect("a status video");
    let async_video =
        block_on_status(&clients, ClientKind::Asynchronous, None, &call).expect("a status video");
    assert_eq!(sync_video, async_video);
    assert_eq!(sync_transport.recorded().len(), 1);
    assert_eq!(async_transport.recorded().len(), 1);
}

#[test]
fn the_sync_and_async_paths_share_the_content_decisions() {
    let sync_transport = FakeTransport::new(vec![Step::Answer(200, b"bytes".to_vec())]);
    let async_transport = FakeTransport::new(vec![Step::Answer(200, b"bytes".to_vec())]);
    let clients = VideoClients {
        sync: &sync_transport,
        asynchronous: &async_transport,
    };
    let reply = content_reply(
        200,
        VideoHeaders::new(),
        br#"{"video": {"url": "https://cdn.fal.run/out/video.mp4"}}"#,
    );
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime builds");
    let sync_body = runtime
        .block_on(content_response(&clients, ClientKind::Sync, &reply))
        .expect("the artifact bytes");
    let async_body = runtime
        .block_on(content_response(&clients, ClientKind::Asynchronous, &reply))
        .expect("the artifact bytes");
    assert_eq!(sync_body, async_body);
    assert_eq!(sync_transport.recorded().len(), 1);
    assert_eq!(async_transport.recorded().len(), 1);
}

#[rstest]
#[case::remix(
    UnsupportedOperation::Remix,
    "video remix is not supported for fal.ai",
    "remix"
)]
#[case::list(
    UnsupportedOperation::List,
    "video listing is not supported for fal.ai",
    "list"
)]
#[case::delete(
    UnsupportedOperation::Delete,
    "video delete is not supported for fal.ai",
    "delete"
)]
#[case::character_create(
    UnsupportedOperation::CreateCharacter,
    "video character creation is not supported for fal.ai",
    "character_create"
)]
#[case::character_get(
    UnsupportedOperation::GetCharacter,
    "video character retrieval is not supported for fal.ai",
    "character_get"
)]
#[case::edit(
    UnsupportedOperation::Edit,
    "video edit is not supported for fal.ai",
    "edit"
)]
#[case::extension(
    UnsupportedOperation::Extension,
    "video extension is not supported for fal.ai",
    "extension"
)]
fn the_unsupported_operations_carry_the_provider_messages(
    #[case] operation: UnsupportedOperation,
    #[case] expected: &str,
    #[case] name: &str,
) {
    assert_eq!(operation.to_string(), expected);
    assert_eq!(operation.name(), name);
    let error = VideoResultsError::from(operation);
    assert_eq!(error.to_string(), expected);
}

#[test]
fn every_unsupported_operation_has_a_name_and_is_listed_once() {
    assert_eq!(
        ALL,
        [
            UnsupportedOperation::Remix,
            UnsupportedOperation::List,
            UnsupportedOperation::Delete,
            UnsupportedOperation::CreateCharacter,
            UnsupportedOperation::GetCharacter,
            UnsupportedOperation::Edit,
            UnsupportedOperation::Extension,
        ]
    );
    assert!(ALL.iter().all(|operation| !operation.name().is_empty()));
}

#[test]
fn an_unforwardable_header_name_is_refused_before_the_probe() {
    let transport = FakeTransport::idle();
    let reply = status_reply(r#"{"status": "COMPLETED"}"#);
    let call = StatusCall {
        request: StatusRequest {
            url: Url::parse(STATUS_URL).expect("the status url parses"),
            headers: headers(&[("bad header", "x")]),
        },
        reply: &reply,
        provider: None,
    };
    let error = run_status(&transport, ClientKind::Sync, &call).expect_err("a bad header");
    assert!(matches!(
        &error,
        VideoResultsError::HeaderName { name, .. } if name.as_str() == "bad header"
    ));
    assert!(transport.recorded().is_empty());
}

#[test]
fn an_unforwardable_header_value_is_refused_before_the_probe() {
    let transport = FakeTransport::idle();
    let reply = status_reply(r#"{"status": "COMPLETED"}"#);
    let call = StatusCall {
        request: StatusRequest {
            url: Url::parse(STATUS_URL).expect("the status url parses"),
            headers: headers(&[("x-trace", "line\nbreak")]),
        },
        reply: &reply,
        provider: None,
    };
    let error = run_status(&transport, ClientKind::Sync, &call).expect_err("a bad header");
    assert!(matches!(
        &error,
        VideoResultsError::HeaderValue { name, .. } if name.as_str() == "x-trace"
    ));
    assert!(transport.recorded().is_empty());
}

#[rstest]
#[case::not_json("not json", false)]
#[case::not_an_object("[]", true)]
fn a_status_reply_that_does_not_decode_is_refused_without_probing(
    #[case] body: &str,
    #[case] not_an_object: bool,
) {
    let transport = FakeTransport::idle();
    let reply = status_reply(body);
    let call = StatusCall {
        request: status_request(),
        reply: &reply,
        provider: None,
    };
    let error = run_status(&transport, ClientKind::Sync, &call).expect_err("an undecodable reply");
    let VideoResultsError::Reply(source) = error else {
        panic!("a reply error");
    };
    assert_eq!(
        matches!(source, FalVideoError::ReplyNotAnObject { .. }),
        not_an_object
    );
    assert!(transport.recorded().is_empty());
}

#[test]
fn the_header_map_lowers_names_and_keeps_values() {
    let map =
        header_map(&headers(&[("Authorization", "Bearer fal-key")])).expect("the headers convert");
    assert_eq!(
        map.get("authorization")
            .and_then(|value| value.to_str().ok()),
        Some("Bearer fal-key")
    );
}

#[test]
fn the_error_text_of_a_decoded_reply_favours_the_detail() {
    let reply = decode_fal_video_reply(
        200,
        &VideoHeaders::new(),
        br#"{"detail": [{"msg": "a"}], "error": "boom"}"#,
    )
    .expect("the reply decodes");
    assert_eq!(error_text(&reply).as_deref(), Some("a"));
    let empty_detail = decode_fal_video_reply(
        200,
        &VideoHeaders::new(),
        br#"{"detail": "", "error": "boom"}"#,
    )
    .expect("the reply decodes");
    assert_eq!(error_text(&empty_detail).as_deref(), Some(""));
}

#[test]
fn a_successful_result_body_never_carries_a_result_error() {
    assert_eq!(result_error(200, br#"{"error": "boom"}"#), None);
    assert_eq!(
        result_error(400, br#"{"error": "boom"}"#).as_deref(),
        Some("boom")
    );
}

#[test]
fn a_status_object_can_be_built_without_a_request() {
    let reply = decode_fal_video_reply(
        200,
        &VideoHeaders::new(),
        br#"{"status": "IN_PROGRESS", "request_id": "req-2"}"#,
    )
    .expect("the reply decodes");
    let video = status_video_object(
        &reply,
        &Url::parse(STATUS_URL).expect("the status url parses"),
        Some("fal_ai"),
        None,
    )
    .expect("a status video");
    assert_eq!(video.status, "in_progress");
    assert_eq!(video.object, VideoObjectType::Video);
}

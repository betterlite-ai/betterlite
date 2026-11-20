use std::time::Duration;

use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_http::proxy::outbound::RequestBody;
use betterlite_http_fixture::request::Request;
use betterlite_models::decisions::Json;
use rstest::rstest;
use secrecy::SecretString;

use super::support::{
    EditPlan, Events, Harness, Reply, SourceCall, UrlSeen, Wire, assert_nothing_sent, body_json,
    expect_created, headers, json_text, object, proxied_client, run, serve_once,
};
use crate::api::bfl::edit::media::SuppliedMedia;
use crate::api::vector_store_http_write::service::ClientKind;
use crate::api::video_edit_transport::calls::Handled;
use crate::api::video_edit_transport::config::RequestFile;
use crate::api::video_edit_transport::edits::VideoEditHandlers as _;
use crate::api::video_edit_transport::error::{VideoClientError, VideoEditTransportError};

#[rstest]
#[case::dod_1_async_hand_off(dod_1_async_hand_off)]
#[case::dod_1_unknown_provider_unconverted(dod_1_unknown_provider_unconverted)]
#[case::dod_2_client_and_headers(dod_2_client_and_headers)]
#[case::dod_2_api_key_argument(dod_2_api_key_argument)]
#[case::dod_2_supplied_client(dod_2_supplied_client)]
#[case::dod_3_prefetch_pair(dod_3_prefetch_pair)]
#[case::dod_3_prefetch_absent(dod_3_prefetch_absent)]
#[case::dod_3_prefetch_timeout(dod_3_prefetch_timeout)]
#[case::dod_4_multipart_files(dod_4_multipart_files)]
#[case::dod_4_empty_files_sends_json(dod_4_empty_files_sends_json)]
#[case::dod_4_json_request_and_response(dod_4_json_request_and_response)]
#[case::dod_5_pre_call_record(dod_5_pre_call_record)]
fn edit_transport_behavior(#[case] check: fn()) {
    check();
}

fn dod_1_async_hand_off() {
    let events = Events::new();
    let wire = Wire::new(Vec::new(), &events);
    let mut harness = Harness::new(&events, wire.clone());
    harness.is_async = true;
    harness.provider = "not-a-provider".to_owned();
    let transport = harness.transport();
    let handled = transport
        .video_edit(harness.edit_request(), &harness.config)
        .unwrap_or_else(|error| panic!("the sync handler failed: {error}"));
    assert!(harness.source.calls().is_empty());
    let Handled::Pending(pending) = handled else {
        panic!("the sync handler ran instead of handing off");
    };
    let error = run(pending).expect_err("the unknown provider fails");
    assert!(matches!(
        error,
        VideoEditTransportError::Client(VideoClientError::UnknownProvider { .. })
    ));
    assert!(harness.config.errors().is_empty());
    assert!(wire.requests().is_empty());
}

fn dod_1_unknown_provider_unconverted() {
    let events = Events::new();
    let server = serve_once("200 fixture", &[], "{}");
    let mut harness = Harness::new(&events, proxied_client());
    harness.provider = "not-a-provider".to_owned();
    harness.config.edit = Some(EditPlan {
        url: format!("http://{}/edits", server.address),
        data: object("{\"prompt\":\"draw\"}"),
        files: None,
        refusal: None,
    });
    let transport = harness.transport();
    let error = run(transport.async_video_edit(harness.edit_request(), &harness.config))
        .expect_err("the unknown provider fails");
    assert!(matches!(
        error,
        VideoEditTransportError::Client(VideoClientError::UnknownProvider { provider })
            if provider == "not-a-provider"
    ));
    assert!(harness.config.errors().is_empty());
    assert_nothing_sent(server);
}

fn dod_2_client_and_headers() {
    let events = Events::new();
    let wire = Wire::new(vec![Reply::json(200, "{}")], &events);
    let mut harness = Harness::new(&events, wire.clone());
    harness
        .params
        .insert("api_key".to_owned(), Json::Text("param-key".to_owned()));
    harness
        .params
        .insert("ssl_verify".to_owned(), Json::Bool(true));
    harness.extra_headers = Some(headers(&[("x-caller", "one"), ("authorization", "caller")]));
    harness.config.validation = headers(&[("authorization", "provider"), ("x-provider", "two")]);
    let transport = harness.transport();
    let created = expect_created(transport.video_edit(harness.edit_request(), &harness.config));
    assert_eq!(created.id, "video-1");
    assert_eq!(
        harness.source.calls(),
        vec![SourceCall {
            mode: ClientKind::Sync,
            provider: String::new(),
            ssl_verify: Some(true),
        }]
    );
    assert_edit_prepare_records(&harness);
}

fn assert_edit_prepare_records(harness: &Harness<Wire>) {
    let validated = harness
        .config
        .validated()
        .expect("validate_environment ran");
    assert_eq!(
        validated.headers,
        headers(&[("x-caller", "one"), ("authorization", "caller")])
    );
    assert_eq!(validated.model, "");
    assert_eq!(validated.api_key, Some("param-key".to_owned()));
    assert_eq!(validated.params, Some(harness.params.clone()));
    assert_eq!(
        harness.config.url_seen(),
        Some(UrlSeen {
            model: String::new(),
            api_base: Some("https://upstream.test".to_owned()),
        })
    );
    let seen = harness
        .config
        .edit_seen()
        .expect("the request transform ran");
    assert_eq!(
        seen.headers,
        headers(&[
            ("x-caller", "one"),
            ("authorization", "caller"),
            ("x-provider", "two"),
        ])
    );
    assert_eq!(seen.api_base, "https://upstream.test");
}

fn dod_2_api_key_argument() {
    let events = Events::new();
    let wire = Wire::new(vec![Reply::json(200, "{}")], &events);
    let mut harness = Harness::new(&events, wire.clone());
    harness
        .params
        .insert("api_key".to_owned(), Json::Text("param-key".to_owned()));
    harness.api_key = Some(SecretString::from("argument-key".to_owned()));
    let transport = harness.transport();
    let created = expect_created(transport.video_edit(harness.edit_request(), &harness.config));
    assert_eq!(created.status, "completed");
    let validated = harness
        .config
        .validated()
        .expect("validate_environment ran");
    assert_eq!(validated.api_key, Some("argument-key".to_owned()));
}

fn dod_2_supplied_client() {
    let events = Events::new();
    let matching = Wire::new(vec![Reply::json(200, "{}")], &events);
    let mut supplied_harness = Harness::new(&events, Wire::new(Vec::new(), &events));
    supplied_harness.supplied = Some((ClientKind::Asynchronous, matching.clone()));
    let supplied_transport = supplied_harness.transport();
    let supplied_object = run(supplied_transport
        .async_video_edit(supplied_harness.edit_request(), &supplied_harness.config))
    .expect("the supplied client answers");
    assert_eq!(supplied_object.id, "video-1");
    assert!(supplied_harness.source.calls().is_empty());
    assert_eq!(matching.requests().len(), 1);
    let mismatched = Wire::new(Vec::new(), &events);
    let pooled = Wire::new(vec![Reply::json(200, "{}")], &events);
    let mut pooled_harness = Harness::new(&events, pooled.clone());
    pooled_harness.supplied = Some((ClientKind::Sync, mismatched.clone()));
    let pooled_transport = pooled_harness.transport();
    let pooled_object =
        run(pooled_transport
            .async_video_edit(pooled_harness.edit_request(), &pooled_harness.config))
        .expect("the pooled client answers");
    assert_eq!(pooled_object.id, "video-1");
    assert_eq!(
        pooled_harness.source.calls(),
        vec![SourceCall {
            mode: ClientKind::Asynchronous,
            provider: "openai".to_owned(),
            ssl_verify: None,
        }]
    );
    assert!(mismatched.requests().is_empty());
    assert_eq!(pooled.requests().len(), 1);
}

fn dod_3_prefetch_pair() {
    let events = Events::new();
    let prefetch_fixture = serve_once(
        "200 fixture",
        &[("content-type", "application/json")],
        "{\"source\":{\"url\":\"https://cdn.test/a.mp4\"}}",
    );
    let main_fixture = serve_once("200 fixture", &[], "{\"id\":\"remote-1\"}");
    let mut harness = Harness::new(&events, proxied_client());
    harness.timeout = Some(Duration::from_secs(9));
    harness.config.validation = headers(&[("x-provider", "two")]);
    harness.config.prefetch = Some((
        format!("http://{}/prefetch", prefetch_fixture.address),
        object("{\"video_id\":\"video-77\"}"),
    ));
    harness.config.edit = Some(EditPlan {
        url: format!("http://{}/edits", main_fixture.address),
        data: object("{\"prompt\":\"draw\"}"),
        files: None,
        refusal: None,
    });
    let transport = harness.transport();
    let created = expect_created(transport.video_edit(harness.edit_request(), &harness.config));
    assert_eq!(created.id, "video-1");
    let prefetch_request = prefetch_fixture
        .finish()
        .expect("the prefetch POST arrived");
    let main_request = main_fixture.finish().expect("the main POST arrived");
    assert_prefetch_wire(&prefetch_request, &main_request);
    let seen = harness
        .config
        .edit_seen()
        .expect("the request transform ran");
    assert_eq!(
        seen.prefetched,
        Some(object("{\"source\":{\"url\":\"https://cdn.test/a.mp4\"}}"))
    );
}

fn assert_prefetch_wire(prefetch: &Request, main: &Request) {
    assert!(prefetch.line.starts_with("POST /prefetch HTTP/1.1"));
    assert!(prefetch.head.contains("x-provider: two"));
    assert_eq!(prefetch.body, "{\"video_id\":\"video-77\"}");
    assert!(main.line.starts_with("POST /edits HTTP/1.1"));
    assert!(main.head.contains("content-type: application/json"));
    assert_eq!(main.body, "{\"prompt\":\"draw\"}");
}

fn dod_3_prefetch_absent() {
    let events = Events::new();
    let wire = Wire::new(vec![Reply::json(200, "{}")], &events);
    let mut harness = Harness::new(&events, wire.clone());
    harness.timeout = Some(Duration::from_secs(2));
    let transport = harness.transport();
    let created = expect_created(transport.video_edit(harness.edit_request(), &harness.config));
    assert_eq!(created.id, "video-1");
    assert_eq!(wire.requests().len(), 1);
    assert!(harness.config.prefetch_seen().is_some());
    let seen = harness
        .config
        .edit_seen()
        .expect("the request transform ran");
    assert_eq!(seen.prefetched, None);
}

fn dod_3_prefetch_timeout() {
    let events = Events::new();
    let wire = Wire::new(
        vec![Reply::json(200, "{}"), Reply::json(200, "{}")],
        &events,
    );
    let mut harness = Harness::new(&events, wire.clone());
    harness.timeout = Some(Duration::from_secs(11));
    harness.config.prefetch = Some((
        "https://upstream.test/prefetch".to_owned(),
        object("{\"video_id\":\"video-77\"}"),
    ));
    let transport = harness.transport();
    let created = expect_created(transport.video_edit(harness.edit_request(), &harness.config));
    assert_eq!(created.id, "video-1");
    let requests = wire.requests();
    assert_eq!(requests.len(), 2);
    let expected = Some(Duration::from_secs(11));
    assert_eq!(requests[0].timeout, expected);
    assert_eq!(requests[1].timeout, expected);
}

fn dod_4_multipart_files() {
    let events = Events::new();
    let wire = Wire::new(vec![Reply::json(200, "{}")], &events);
    let mut harness = Harness::new(&events, wire.clone());
    harness.timeout = Some(Duration::from_secs(5));
    harness.video_file = Some(SuppliedMedia::Bytes(b"source".to_vec()));
    harness.config.edit = Some(EditPlan {
        url: "https://upstream.test/edits".to_owned(),
        data: object(
            "{\"prompt\":\"draw\",\"enabled\":true,\"disabled\":false,\"missing\":null,\"count\":7,\"ratio\":0.5,\"tags\":[\"a\",2]}",
        ),
        files: Some(vec![RequestFile {
            field: "video".to_owned(),
            filename: Some("clip.mp4".to_owned()),
            content: b"MP4".to_vec(),
            content_type: Some("video/mp4".to_owned()),
        }]),
        refusal: None,
    });
    let transport = harness.transport();
    let created = expect_created(transport.video_edit(harness.edit_request(), &harness.config));
    assert_eq!(created.id, "video-1");
    let seen = harness
        .config
        .edit_seen()
        .expect("the request transform ran");
    assert!(seen.video_file);
    let requests = wire.requests();
    let request = requests.first().expect("the main POST was sent");
    assert_eq!(request.timeout, Some(Duration::from_secs(5)));
    let RequestBody::Multipart { boundary, parts } = &request.body else {
        panic!("the request is not multipart");
    };
    assert!(!boundary.is_empty());
    assert_multipart_data(parts);
    assert_video_part(parts.last().expect("the video part"));
}

fn assert_multipart_data(parts: &[MultipartPart]) {
    let fields = [
        ("prompt", "draw"),
        ("enabled", "true"),
        ("disabled", "false"),
        ("missing", ""),
        ("count", "7"),
        ("ratio", "0.5"),
        ("tags", "a"),
        ("tags", "2"),
    ];
    // prompt, five scalars, the two expanded tags and the video file
    assert_eq!(parts.len(), 9);
    for (part, (name, text)) in parts.iter().zip(fields) {
        assert_eq!(part.name, name);
        assert_eq!(part.data, text.as_bytes());
        assert!(part.file_name.is_none());
    }
}

fn assert_video_part(part: &MultipartPart) {
    assert_eq!(part.name, "video");
    assert_eq!(part.file_name.as_deref(), Some("clip.mp4"));
    assert_eq!(part.content_type.as_deref(), Some("video/mp4"));
    assert_eq!(part.data, b"MP4".to_vec());
}

fn dod_4_empty_files_sends_json() {
    let events = Events::new();
    let wire = Wire::new(vec![Reply::json(200, "{}")], &events);
    let mut harness = Harness::new(&events, wire.clone());
    harness.config.edit = Some(EditPlan {
        url: "https://upstream.test/edits".to_owned(),
        data: object("{\"prompt\":\"draw\"}"),
        files: Some(Vec::new()),
        refusal: None,
    });
    let transport = harness.transport();
    let created = expect_created(transport.video_edit(harness.edit_request(), &harness.config));
    assert_eq!(created.id, "video-1");
    let requests = wire.requests();
    let request = requests.first().expect("the main POST was sent");
    assert!(matches!(&request.body, RequestBody::Json(_)));
    assert_eq!(body_json(request), json_text("{\"prompt\":\"draw\"}"));
}

fn dod_4_json_request_and_response() {
    let events = Events::new();
    let wire = Wire::new(vec![Reply::json(200, "{\"id\":\"remote-1\"}")], &events);
    let mut harness = Harness::new(&events, wire.clone());
    harness.timeout = Some(Duration::from_secs(3));
    harness.log.object = object("{\"call_type\":\"avideo_edit\"}");
    harness.config.edit = Some(EditPlan {
        url: "https://upstream.test/edits".to_owned(),
        data: object("{\"prompt\":\"draw\"}"),
        files: None,
        refusal: None,
    });
    let transport = harness.transport();
    let created = expect_created(transport.video_edit(harness.edit_request(), &harness.config));
    assert_eq!(created.id, "video-1");
    let requests = wire.requests();
    let request = requests.first().expect("the main POST was sent");
    assert_eq!(request.timeout, Some(Duration::from_secs(3)));
    assert_eq!(body_json(request), json_text("{\"prompt\":\"draw\"}"));
    let reply = harness
        .config
        .edit_reply_seen()
        .expect("the response transform ran");
    assert_eq!(reply.status, 200);
    assert_eq!(reply.body, b"{\"id\":\"remote-1\"}".to_vec());
    assert_eq!(reply.provider.as_deref(), Some("openai"));
    assert_eq!(reply.logging, object("{\"call_type\":\"avideo_edit\"}"));
    assert_eq!(reply.request_data, Some(object("{\"prompt\":\"draw\"}")));
}

fn dod_5_pre_call_record() {
    let events = Events::new();
    let wire = Wire::new(vec![Reply::json(200, "{}")], &events);
    let mut harness = Harness::new(&events, wire.clone());
    harness.extra_body = Some(object("{\"seed\":7}"));
    harness.config.validation = headers(&[("x-provider", "two")]);
    harness.config.edit = Some(EditPlan {
        url: "https://upstream.test/edits".to_owned(),
        data: object("{\"prompt\":\"draw\"}"),
        files: None,
        refusal: None,
    });
    let transport = harness.transport();
    let created = expect_created(transport.video_edit(harness.edit_request(), &harness.config));
    assert_eq!(created.id, "video-1");
    let calls = harness.log.calls();
    assert_eq!(calls.len(), 1);
    let call = calls.first().expect("the pre_call ran");
    assert_eq!(call.input, "make the sky brighter");
    assert_eq!(call.api_key, "");
    assert_eq!(call.complete_input_dict, object("{\"prompt\":\"draw\"}"));
    assert_eq!(call.api_base, "https://upstream.test/edits");
    assert_eq!(call.headers, headers(&[("x-provider", "two")]));
    assert_eq!(call.video_id, "video-77");
    assert_eq!(
        events.recorded().join(","),
        "edit-transform,pre-call,send,edit-response"
    );
    let seen = harness
        .config
        .edit_seen()
        .expect("the request transform ran");
    assert_eq!(seen.extra_body, Some(object("{\"seed\":7}")));
}

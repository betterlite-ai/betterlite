use std::time::Duration;

use betterlite_http_fixture::request::Request;
use betterlite_models::decisions::Json;
use rstest::rstest;
use secrecy::SecretString;

use super::support::{
    Events, ExtensionPlan, ExtensionSeen, Harness, Reply, ReplySeen, SourceCall, Wire,
    assert_nothing_sent, body_json, expect_created, headers, json_text, object, proxied_client,
    run, serve_once,
};
use crate::api::vector_store_http_write::service::ClientKind;
use crate::api::video_edit_transport::calls::Handled;
use crate::api::video_edit_transport::error::{VideoClientError, VideoEditTransportError};

#[rstest]
#[case::dod_1_async_hand_off(dod_1_async_hand_off)]
#[case::dod_1_unknown_provider_unconverted(dod_1_unknown_provider_unconverted)]
#[case::dod_2_client_and_headers(dod_2_client_and_headers)]
#[case::dod_2_api_key_argument(dod_2_api_key_argument)]
#[case::dod_2_supplied_client_matches(dod_2_supplied_client_matches)]
#[case::dod_6_transform_and_send(dod_6_transform_and_send)]
#[case::dod_6_pre_call_record(dod_6_pre_call_record)]
#[case::dod_6_json_post_timeout(dod_6_json_post_timeout)]
fn extension_transport_behavior(#[case] check: fn()) {
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
        .video_extension(harness.extension_request(), &harness.config)
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
    harness.config.extension = Some(ExtensionPlan {
        url: format!("http://{}/extensions", server.address),
        data: object("{\"seconds\":\"4\"}"),
        refusal: None,
    });
    let transport = harness.transport();
    let error = run(transport.async_video_extension(harness.extension_request(), &harness.config))
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
        .insert("ssl_verify".to_owned(), Json::Bool(false));
    harness.extra_headers = Some(headers(&[("x-caller", "one")]));
    harness.config.validation = headers(&[("x-provider", "two")]);
    let transport = harness.transport();
    let created =
        run(transport.async_video_extension(harness.extension_request(), &harness.config))
            .expect("the extension runs");
    assert_eq!(created.id, "video-1");
    assert_eq!(
        harness.source.calls(),
        vec![SourceCall {
            mode: ClientKind::Asynchronous,
            provider: "openai".to_owned(),
            ssl_verify: Some(false),
        }]
    );
    assert_extension_prepare_records(&harness);
}

fn assert_extension_prepare_records(harness: &Harness<Wire>) {
    let validated = harness
        .config
        .validated()
        .expect("validate_environment ran");
    assert_eq!(validated.headers, headers(&[("x-caller", "one")]));
    assert_eq!(validated.model, "");
    assert_eq!(
        harness
            .config
            .url_seen()
            .expect("get_complete_url ran")
            .model,
        ""
    );
    let seen = harness
        .config
        .extension_seen()
        .expect("the request transform ran");
    assert_eq!(
        seen.headers,
        headers(&[("x-caller", "one"), ("x-provider", "two")])
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
    let created =
        run(transport.async_video_extension(harness.extension_request(), &harness.config))
            .expect("the extension runs");
    assert_eq!(created.id, "video-1");
    let validated = harness
        .config
        .validated()
        .expect("validate_environment ran");
    assert_eq!(validated.api_key, Some("argument-key".to_owned()));
}

fn dod_2_supplied_client_matches() {
    let events = Events::new();
    let supplied = Wire::new(vec![Reply::json(200, "{}")], &events);
    let mut harness = Harness::new(&events, Wire::new(Vec::new(), &events));
    harness.supplied = Some((ClientKind::Asynchronous, supplied.clone()));
    let transport = harness.transport();
    let created =
        run(transport.async_video_extension(harness.extension_request(), &harness.config))
            .expect("the supplied client answers");
    assert_eq!(created.id, "video-1");
    assert!(harness.source.calls().is_empty());
    assert_eq!(supplied.requests().len(), 1);
}

fn dod_6_transform_and_send() {
    let events = Events::new();
    let server = serve_once("200 fixture", &[], "{\"id\":\"remote-2\"}");
    let mut harness = Harness::new(&events, proxied_client());
    harness.timeout = Some(Duration::from_secs(4));
    harness.extra_body = Some(object("{\"seed\":7}"));
    harness.log.object = object("{\"call_type\":\"avideo_extension\"}");
    harness.config.validation = headers(&[("x-provider", "two")]);
    harness.config.extension = Some(ExtensionPlan {
        url: format!("http://{}/extensions", server.address),
        data: object("{\"seconds\":4,\"prompt\":\"extend\"}"),
        refusal: None,
    });
    let transport = harness.transport();
    let created =
        expect_created(transport.video_extension(harness.extension_request(), &harness.config));
    assert_eq!(created.id, "video-1");
    let seen = harness
        .config
        .extension_seen()
        .expect("the request transform ran");
    assert_extension_transform(&seen);
    let request = server.finish().expect("the extension POST arrived");
    let reply = harness
        .config
        .extension_reply_seen()
        .expect("the response transform ran");
    assert_extension_wire(&request, &reply);
    assert_eq!(
        events.recorded().join(","),
        "extension-transform,pre-call,send,extension-response"
    );
}

fn assert_extension_transform(seen: &ExtensionSeen) {
    assert_eq!(seen.prompt, "make the sky brighter");
    assert_eq!(seen.video_id, "video-77");
    assert_eq!(seen.seconds, "4");
    assert_eq!(seen.extra_body, Some(object("{\"seed\":7}")));
    assert_eq!(seen.api_base, "https://upstream.test");
}

fn assert_extension_wire(request: &Request, reply: &ReplySeen) {
    assert!(request.line.starts_with("POST /extensions HTTP/1.1"));
    assert!(request.head.contains("x-provider: two"));
    assert!(request.head.contains("content-type: application/json"));
    assert_eq!(request.body, "{\"seconds\":4,\"prompt\":\"extend\"}");
    assert_eq!(reply.status, 200);
    assert_eq!(reply.body, b"{\"id\":\"remote-2\"}".to_vec());
    assert_eq!(reply.provider.as_deref(), Some("openai"));
    assert_eq!(
        reply.logging,
        object("{\"call_type\":\"avideo_extension\"}")
    );
}

fn dod_6_pre_call_record() {
    let events = Events::new();
    let wire = Wire::new(vec![Reply::json(200, "{}")], &events);
    let mut harness = Harness::new(&events, wire.clone());
    harness.config.validation = headers(&[("x-provider", "two")]);
    harness.config.extension = Some(ExtensionPlan {
        url: "https://upstream.test/extensions".to_owned(),
        data: object("{\"seconds\":\"4\"}"),
        refusal: None,
    });
    let transport = harness.transport();
    let created =
        expect_created(transport.video_extension(harness.extension_request(), &harness.config));
    assert_eq!(created.id, "video-1");
    let calls = harness.log.calls();
    assert_eq!(calls.len(), 1);
    let call = calls.first().expect("the pre_call ran");
    assert_eq!(call.input, "make the sky brighter");
    assert_eq!(call.api_key, "");
    assert_eq!(call.complete_input_dict, object("{\"seconds\":\"4\"}"));
    assert_eq!(call.api_base, "https://upstream.test/extensions");
    assert_eq!(call.headers, headers(&[("x-provider", "two")]));
    assert_eq!(call.video_id, "video-77");
    assert_eq!(
        events.recorded().join(","),
        "extension-transform,pre-call,send,extension-response"
    );
}

fn dod_6_json_post_timeout() {
    let events = Events::new();
    let wire = Wire::new(vec![Reply::json(200, "{}")], &events);
    let mut harness = Harness::new(&events, wire.clone());
    harness.timeout = Some(Duration::from_secs(7));
    let transport = harness.transport();
    assert_eq!(
        expect_created(transport.video_extension(harness.extension_request(), &harness.config)).id,
        "video-1"
    );
    let requests = wire.requests();
    let request = requests.first().expect("the extension POST was sent");
    assert_eq!(request.timeout, Some(Duration::from_secs(7)));
    assert_eq!(body_json(request), json_text("{\"seconds\":\"4\"}"));
}

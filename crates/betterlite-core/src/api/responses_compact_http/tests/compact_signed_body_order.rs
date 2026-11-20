use std::time::Duration;

use betterlite_http::proxy::outbound::RequestBody;
use betterlite_models::decisions::Json as LoggedJson;
use betterlite_openai::response_events::ReplyBody;
use betterlite_prompts::arguments::json::Json as PromptJson;
use secrecy::ExposeSecret as _;

use super::support::{
    COMPACT_OK, Events, Fixture, ITEM_INPUT, NORMALIZED_BODY, RecordingConfig, Reply, Source,
    TestCall, Wire, body_bytes, body_json, caller_headers, compact_deps, items_input, json,
    keyed_params, logged, optional_params,
};
use crate::api::responses_compact_http::service::ClientKind;

const ASYNC_EVENTS: [&str; 9] = [
    "validate",
    "url",
    "transform",
    "normalize",
    "off_loop",
    "sign",
    "log",
    "send",
    "response",
];

pub(super) async fn sync_calls() {
    let events = Events::new();
    let config = RecordingConfig::new("http://mock.test/v1/responses", &events);
    config.queue_signed(None);
    config.queue_signed(Some(b"signed".to_vec()));
    config.queue_signed(Some(Vec::new()));
    let wire = Wire::replying(Reply::json(200, COMPACT_OK), &events);
    let source = Source::new(wire.clone());
    let deps = compact_deps(&config, &source);
    let params = keyed_params("http://mock.test/v1", "sk-key");
    let input = items_input(ITEM_INPUT);
    let optional = optional_params(r#"{"instructions":"short"}"#);
    let headers = caller_headers(&[("X-Trace", "trace-a")]);
    let mut call = TestCall::new("gpt-4o", &input, &optional, &params);
    call.extra_headers = Some(&headers);
    call.timeout = Some(Duration::from_secs(7));
    let mut fixture = Fixture::new("responses");
    for expected in [
        "the signed compact call succeeds",
        "the signed-byte call succeeds",
        "the empty signed-byte call succeeds",
    ] {
        let reply = fixture
            .run(&deps, ClientKind::Sync, &call.call(), &events)
            .await
            .expect(expected);
        assert!(matches!(reply.response, ReplyBody::Typed(_)));
    }
    assert!(!events.recorded().contains(&String::from("off_loop")));
    signs(&config);
    pre_call_logs(&fixture);
    sent_requests(&wire);
}

fn signs(config: &RecordingConfig) {
    let signs = config.signs();
    assert_eq!(signs.len(), 3);
    let recorded = &signs[0];
    assert_eq!(recorded.api_base, "http://mock.test/v1/responses/compact");
    assert_eq!(recorded.model.as_deref(), Some("gpt-4o"));
    assert_eq!(recorded.api_key.as_deref(), Some("sk-key"));
    assert_eq!(
        recorded.optional_params.api_base.as_deref(),
        Some("http://mock.test/v1")
    );
    assert_eq!(
        recorded
            .optional_params
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned())
            .as_deref(),
        Some("sk-key")
    );
    assert_eq!(recorded.optional_params.ssl_verify, None);
    let PromptJson::Object(expected_data) = json(NORMALIZED_BODY) else {
        panic!("the normalized body is an object");
    };
    assert_eq!(recorded.data, expected_data);
}

fn pre_call_logs(fixture: &Fixture) {
    assert_eq!(
        fixture.logged("complete_input_dict"),
        Some(logged(NORMALIZED_BODY))
    );
    let Some(LoggedJson::Object(logged_headers)) = fixture.logged("headers") else {
        panic!("the pre-call logs the signed headers");
    };
    assert_eq!(
        logged_headers.get("X-Signed"),
        Some(&LoggedJson::Text(String::from("yes")))
    );
    assert_eq!(
        logged_headers.get("X-Trace"),
        Some(&LoggedJson::Text(String::from("trace-a")))
    );
    assert_eq!(
        logged_headers.get("Authorization"),
        Some(&LoggedJson::Text(String::from("Be****er")))
    );
}

fn sent_requests(wire: &Wire) {
    let requests = wire.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(body_json(&requests[0]), json(NORMALIZED_BODY));
    assert_eq!(
        requests[0].url.as_str(),
        "http://mock.test/v1/responses/compact"
    );
    assert_eq!(
        requests[0]
            .headers
            .get("x-signed")
            .and_then(|value| value.to_str().ok()),
        Some("yes")
    );
    assert_eq!(body_bytes(&requests[1]), Some(b"signed".as_slice()));
    assert!(matches!(
        &requests[1].body,
        RequestBody::Bytes { content_type, data }
            if data == b"signed"
                && content_type.to_str().ok() == Some("application/octet-stream")
    ));
    assert!(matches!(
        &requests[2].body,
        RequestBody::Bytes { data, .. } if data.is_empty()
    ));
}

pub(super) async fn async_call() {
    let events = Events::new();
    let config = RecordingConfig::new("http://mock.test/v1/responses", &events);
    config.queue_signed(None);
    let wire = Wire::replying(Reply::json(200, COMPACT_OK), &events);
    let source = Source::new(wire.clone());
    let deps = compact_deps(&config, &source);
    let params = keyed_params("http://mock.test/v1", "sk-key");
    let input = items_input(ITEM_INPUT);
    let optional = optional_params(r#"{"instructions":"short"}"#);
    let headers = caller_headers(&[("X-Trace", "trace-a")]);
    let mut call = TestCall::new("gpt-4o", &input, &optional, &params);
    call.extra_headers = Some(&headers);
    call.timeout = Some(Duration::from_secs(7));
    let mut fixture = Fixture::new("responses");
    fixture
        .run(&deps, ClientKind::Asynchronous, &call.call(), &events)
        .await
        .expect("the async compact call signs off the loop");
    assert_eq!(events.recorded(), ASYNC_EVENTS);
}

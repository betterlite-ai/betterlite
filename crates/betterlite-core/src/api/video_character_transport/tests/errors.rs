use betterlite_http::interactions::base::{ClientError, RequestFailure, ResponseHeaders};
use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::controls::json_value::JsonValue;
use http::header::HeaderValue;

use super::support::{
    FakeWire, Fixture, Seen, TestError, create_call, extra_headers, get_call, params, secret, video,
};
use crate::api::video_character_transport::error::CharacterError;
use crate::api::video_character_transport::service::CharacterTransport;

const BASE: &str = "https://api.example/v1";

#[test]
fn creation_turns_a_failing_status_into_the_provider_error() {
    let mut fixture = Fixture::new();
    fixture.wire.status = 503;
    fixture.wire.body = b"upstream is down".to_vec();
    fixture
        .wire
        .headers
        .append("retry-after", HeaderValue::from_static("7"));
    let litellm = params(&[("api_base", BASE)]);
    let clip = video();
    let call = create_call("Wren", &clip, &litellm);

    let error = fixture.sync_create(&call, None).unwrap_err();

    let CharacterError::Provider(TestError {
        message,
        status_code,
        headers,
    }) = error
    else {
        panic!("a failing status becomes the provider's error");
    };
    assert_eq!(message, "upstream is down");
    assert_eq!(status_code, 503);
    assert_eq!(headers.get("retry-after").map(String::as_str), Some("7"));
    assert_eq!(
        fixture.config.errors(),
        vec![(String::from("upstream is down"), 503, headers)]
    );
    assert_eq!(
        fixture.events.recorded().last().map(String::as_str),
        Some("send")
    );
}

#[test]
fn lookup_turns_a_failing_status_into_the_provider_error() {
    let mut fixture = Fixture::new();
    fixture.wire.status = 404;
    fixture.wire.body = b"no such character".to_vec();
    let litellm = params(&[("api_base", BASE)]);
    let call = get_call("char-1", &litellm);

    let error = fixture.sync_get(&call, None).unwrap_err();

    let CharacterError::Provider(TestError {
        message,
        status_code,
        ..
    }) = error
    else {
        panic!("a failing status becomes the provider's error");
    };
    assert_eq!(message, "no such character");
    assert_eq!(status_code, 404);
    assert_eq!(
        fixture.events.recorded().last().map(String::as_str),
        Some("send")
    );
}

fn refused_statuses(status: u16) {
    let mut fixture = Fixture::new();
    fixture.wire.status = status;
    fixture.wire.body = b"no".to_vec();
    let litellm = params(&[("api_base", BASE)]);
    let clip = video();
    let call = create_call("Wren", &clip, &litellm);

    let error = fixture.sync_create(&call, None).unwrap_err();

    assert!(matches!(&error, CharacterError::Provider(_)), "{error:?}");
    assert_eq!(fixture.config.errors().len(), 1);
}

fn accepted_statuses(status: u16) {
    let mut fixture = Fixture::new();
    fixture.wire.status = status;
    let litellm = params(&[("api_base", BASE)]);
    let clip = video();
    let call = create_call("Wren", &clip, &litellm);

    fixture.sync_create(&call, None).unwrap();

    assert_eq!(
        fixture.events.recorded().last().map(String::as_str),
        Some("transform-create-response")
    );
}

#[test]
fn statuses_outside_2xx_are_failures() {
    refused_statuses(199);
    refused_statuses(300);
    refused_statuses(400);
    refused_statuses(599);
}

#[test]
fn two_hundred_and_two_ninety_nine_are_accepted() {
    accepted_statuses(200);
    accepted_statuses(299);
}

#[test]
fn creation_passes_a_send_failure_through() {
    let mut fixture = Fixture::new();
    let events = fixture.events.clone();
    fixture.use_wire(FakeWire::failing(
        &events,
        RequestFailure::BodyTooLarge { limit: 4 },
    ));
    let litellm = params(&[("api_base", BASE)]);
    let clip = video();
    let call = create_call("Wren", &clip, &litellm);

    let error = fixture.sync_create(&call, None).unwrap_err();

    let CharacterError::Transport(RequestFailure::BodyTooLarge { limit }) = error else {
        panic!("a send failure keeps its typed reason");
    };
    assert_eq!(limit, 4);
    assert_eq!(fixture.config.errors().len(), 0);
    assert_eq!(
        fixture.events.recorded().last().map(String::as_str),
        Some("send")
    );
}

#[test]
fn creation_turns_a_transform_failure_into_a_synthesized_provider_error() {
    let mut fixture = Fixture::new();
    fixture.config.reply_error = Some(String::from("the reply is not JSON"));
    let litellm = params(&[("api_base", BASE)]);
    let clip = video();
    let call = create_call("Wren", &clip, &litellm);

    let error = fixture.sync_create(&call, None).unwrap_err();

    let CharacterError::Provider(TestError {
        message,
        status_code,
        headers,
    }) = error
    else {
        panic!("a transform failure reads as a synthesized provider error");
    };
    assert_eq!(message, "the reply is not JSON");
    assert_eq!(status_code, 500);
    assert!(headers.is_empty());
    assert_eq!(
        fixture.config.errors(),
        vec![(
            String::from("the reply is not JSON"),
            500,
            ResponseHeaders::new()
        )]
    );
}

#[test]
fn lookup_turns_a_transform_failure_into_a_synthesized_provider_error() {
    let mut fixture = Fixture::new();
    fixture.config.reply_error = Some(String::from("the reply names no id"));
    let litellm = params(&[("api_base", BASE)]);
    let call = get_call("char-1", &litellm);

    let error = fixture.sync_get(&call, None).unwrap_err();

    let CharacterError::Provider(TestError {
        message,
        status_code,
        ..
    }) = error
    else {
        panic!("a transform failure reads as a synthesized provider error");
    };
    assert_eq!(message, "the reply names no id");
    assert_eq!(status_code, 500);
}

#[test]
fn a_request_transform_failure_is_typed_before_any_log_or_send() {
    let mut fixture = Fixture::new();
    fixture.config.request_error = Some(String::from("the base is missing"));
    let litellm = params(&[("api_base", BASE)]);
    let clip = video();
    let call = create_call("Wren", &clip, &litellm);

    let error = fixture.sync_create(&call, None).unwrap_err();

    let CharacterError::Provider(TestError {
        message,
        status_code,
        ..
    }) = error
    else {
        panic!("a request transform failure keeps the provider's error");
    };
    assert_eq!(message, "the base is missing");
    assert_eq!(status_code, 0);
    assert_eq!(
        fixture.events.recorded(),
        ["select-sync", "validate", "url", "transform-create"]
    );
    assert_eq!(fixture.config.errors().len(), 0);
}

#[test]
fn a_complete_url_failure_is_typed_before_the_transform() {
    let mut fixture = Fixture::new();
    fixture.config.url_error = Some(String::from("api_base is required"));
    let litellm = params(&[]);
    let clip = video();
    let call = create_call("Wren", &clip, &litellm);

    let error = fixture.sync_create(&call, None).unwrap_err();

    let CharacterError::Provider(TestError { message, .. }) = error else {
        panic!("a complete-url failure keeps the provider's error");
    };
    assert_eq!(message, "api_base is required");
    assert_eq!(
        fixture.events.recorded(),
        ["select-sync", "validate", "url"]
    );
}

#[test]
fn an_unusable_api_base_type_names_the_field() {
    let mut fixture = Fixture::new();
    let mut litellm = params(&[]);
    litellm.insert("api_base".to_owned(), JsonValue::Bool(true));
    let clip = video();
    let call = create_call("Wren", &clip, &litellm);

    let error = fixture.sync_create(&call, None).unwrap_err();

    let CharacterError::Client(ClientError::FieldType { field, expected }) = error else {
        panic!("a non-string api_base is a typed parameter error");
    };
    assert_eq!(field, "api_base");
    assert_eq!(expected, "a string");
    assert_eq!(fixture.events.recorded(), ["select-sync", "validate"]);
}

#[test]
fn an_unusable_api_key_type_names_the_field() {
    let mut fixture = Fixture::new();
    let mut litellm = params(&[]);
    litellm.insert("api_key".to_owned(), JsonValue::Bool(true));
    let clip = video();
    let call = create_call("Wren", &clip, &litellm);

    let error = fixture.sync_create(&call, None).unwrap_err();

    let CharacterError::Client(ClientError::FieldType { field, .. }) = error else {
        panic!("a non-string api_key is a typed parameter error");
    };
    assert_eq!(field, "api_key");
    assert_eq!(fixture.events.recorded(), ["select-sync"]);
}

#[test]
fn a_caller_api_key_short_circuits_the_params_one() {
    let mut fixture = Fixture::new();
    let mut litellm = params(&[("api_base", BASE)]);
    litellm.insert("api_key".to_owned(), JsonValue::Bool(true));
    let key = secret("sk-caller");
    let clip = video();
    let mut call = create_call("Wren", &clip, &litellm);
    call.api_key = Some(&key);

    fixture.sync_create(&call, None).unwrap();

    assert_eq!(
        fixture.config.seen()[0],
        Seen::Environment {
            api_key: Some(String::from("sk-caller")),
            model: String::new(),
            headers: Vec::new(),
        }
    );
}

#[test]
fn a_pre_call_failure_surfaces_before_the_send() {
    let mut fixture = Fixture::new();
    fixture.state.request_logging.raw_request_only = true;
    let litellm = params(&[("api_base", BASE)]);
    let clip = video();
    let call = create_call("Wren", &clip, &litellm);

    let error = fixture.sync_create(&call, None).unwrap_err();

    let CharacterError::Logging(PrePostError::RawRequestCaptured) = error else {
        panic!("the captured raw request stops the call");
    };
    assert_eq!(
        fixture.events.recorded(),
        ["select-sync", "validate", "url", "transform-create", "log",]
    );
}

#[test]
fn a_multipart_part_that_breaks_the_boundary_is_refused() {
    let mut fixture = Fixture::new();
    fixture.config.files = Some(vec![MultipartPart {
        name: String::from("video"),
        file_name: None,
        content_type: None,
        data: b"--fixed--".to_vec(),
    }]);
    let litellm = params(&[("api_base", BASE)]);
    let clip = video();
    let call = create_call("Wren", &clip, &litellm);
    let env = fixture.log.env();
    let mut transport = CharacterTransport::with_generated_boundary(
        &fixture.config,
        &mut fixture.clients,
        &mut fixture.state,
        &env,
    );
    transport.boundary = || String::from("fixed");

    let error = match transport.video_create_character_handler(&call) {
        Ok(_handled) => panic!("the part would break the body"),
        Err(error) => error,
    };

    let CharacterError::Multipart(TransportError::MultipartBoundary { boundary }) = error else {
        panic!("a part holding the boundary is refused");
    };
    assert_eq!(boundary, "fixed");
    assert_eq!(
        fixture.events.recorded(),
        ["select-sync", "validate", "url", "transform-create", "log",]
    );
}

#[test]
fn a_header_that_cannot_be_sent_is_refused() {
    let mut fixture = Fixture::new();
    let litellm = params(&[("api_base", BASE)]);
    let caller_headers = extra_headers(&[("x-bad", "line\nbreak")]);
    let clip = video();
    let mut call = create_call("Wren", &clip, &litellm);
    call.extra_headers = Some(&caller_headers);

    let error = fixture.sync_create(&call, None).unwrap_err();

    let CharacterError::Transport(RequestFailure::HeaderValue(_)) = error else {
        panic!("a header with a newline is refused");
    };
}

#[test]
fn a_url_the_transform_returns_unparseable_is_refused() {
    let mut fixture = Fixture::new();
    fixture.config.create_url = String::from("not a url");
    let litellm = params(&[("api_base", BASE)]);
    let clip = video();
    let call = create_call("Wren", &clip, &litellm);

    let error = fixture.sync_create(&call, None).unwrap_err();

    let CharacterError::Transport(RequestFailure::Url(_)) = error else {
        panic!("the transform's url is parsed before the send");
    };
    assert_eq!(
        fixture.events.recorded(),
        ["select-sync", "validate", "url", "transform-create", "log",]
    );
}

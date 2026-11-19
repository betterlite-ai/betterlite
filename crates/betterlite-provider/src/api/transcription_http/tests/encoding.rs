use betterlite_http::proxy::outbound::RequestBody;
use betterlite_models::decisions::Json;
use betterlite_openai::api::transcription_contract::{
    AudioTranscriptionData, AudioTranscriptionFileValue, AudioTranscriptionFiles,
    AudioTranscriptionRequestData,
};
use betterlite_openai::openai_base_config::Object;

use super::support::{
    Entry, Events, Fixture, RecordingConfig, Reply, Source, Wire, body_bytes, body_json, call,
    deps, header, json, object, record,
};
use crate::api::transcription_http::config::Headers;
use crate::api::transcription_http::wire::{TranscriptionBody, TranscriptionRequest};

#[tokio::test]
async fn transcription_http_encoding() {
    assert_json_body();
    assert_empty_object_data_is_json();
    assert_empty_files_selects_form();
    assert_empty_files_and_fields_send_no_body();
    assert_form_repeats_list_values();
    assert_named_files_are_multipart();
    assert_declared_boundary_is_kept();
    assert_declared_type_without_boundary_gets_one();
    assert_bytes_are_raw();
    assert_content_type_override();
    assert_validation_and_url_arguments();
    assert_logging_input();
    assert_logging_complete_input();
    assert_logged_headers_are_masked();
}

async fn send(
    data: AudioTranscriptionRequestData,
    optional: &Object,
) -> (TranscriptionRequest, Fixture, RecordingConfig, Events, Wire) {
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, r#"{"text":"hi"}"#), &events);
    let source = Source::new(wire.clone());
    let config = RecordingConfig::new(data, &events);
    let litellm = object("{}");
    let request_call = call(optional, &litellm);
    let mut fixture = Fixture::new("transcription");
    let reply = fixture
        .run(
            &events,
            &deps(&config, &source),
            &request_call,
            Entry::Audio,
        )
        .await
        .expect("the fixture call succeeds");
    assert_eq!(reply.response.text.as_deref(), Some("hi"));
    let mut requests = wire.requests();
    assert_eq!(requests.len(), 1);
    let request = requests.remove(0);
    drop(requests);
    (request, fixture, config, events, wire)
}

async fn assert_json_body() {
    let optional = object(r#"{"query":"hello"}"#);
    let (request, _fixture, _config, _events, _wire) = send(
        record(AudioTranscriptionData::Object(object(
            r#"{"query":"hello"}"#,
        ))),
        &optional,
    )
    .await;
    assert_eq!(
        request.url.as_str(),
        "https://fixture.test/v1/audio/transcriptions"
    );
    assert_eq!(body_json(&request.body), json(r#"{"query":"hello"}"#));
    assert_eq!(
        header(&request.headers, "authorization"),
        Some("Bearer sk-provider")
    );
    assert_eq!(header(&request.headers, "content-type"), None);
}

async fn assert_empty_object_data_is_json() {
    let optional = object("{}");
    let (request, _fixture, _config, _events, _wire) = send(
        record(AudioTranscriptionData::Object(Object::new())),
        &optional,
    )
    .await;
    assert!(matches!(
        &request.body,
        TranscriptionBody::Encoded(RequestBody::Json(_))
    ));
    assert_eq!(body_json(&request.body), json("{}"));
}

async fn assert_empty_files_selects_form() {
    let optional = object(r#"{"query":"hello"}"#);
    let mut data = record(AudioTranscriptionData::Object(object(
        r#"{"query":"hello"}"#,
    )));
    data.files = Some(AudioTranscriptionFiles::new());
    let (request, _fixture, _config, _events, _wire) = send(data, &optional).await;
    let TranscriptionBody::Encoded(RequestBody::Bytes { content_type, data }) = &request.body
    else {
        panic!("the empty files case sends a form");
    };
    assert_eq!(data, b"query=hello");
    assert_eq!(
        content_type.to_str().expect("the content type is text"),
        "application/x-www-form-urlencoded"
    );
}

async fn assert_empty_files_and_fields_send_no_body() {
    let optional = object("{}");
    let mut data = record(AudioTranscriptionData::Object(Object::new()));
    data.files = Some(AudioTranscriptionFiles::new());
    let (request, _fixture, _config, _events, _wire) = send(data, &optional).await;
    assert!(matches!(
        &request.body,
        TranscriptionBody::Encoded(RequestBody::Empty)
    ));
}

async fn assert_form_repeats_list_values() {
    let optional = object(r#"{"tag":["a","b"]}"#);
    let mut data = record(AudioTranscriptionData::Object(object(
        r#"{"tag":["a","b"]}"#,
    )));
    data.files = Some(AudioTranscriptionFiles::new());
    let (request, _fixture, _config, _events, _wire) = send(data, &optional).await;
    let bytes = body_bytes(&request.body).expect("the form carries bytes");
    assert_eq!(bytes, b"tag=a&tag=b");
}

async fn assert_named_files_are_multipart() {
    let optional = object(r#"{"query":"hello"}"#);
    let mut data = record(AudioTranscriptionData::Object(object(
        r#"{"query":"hello"}"#,
    )));
    data.files = Some(AudioTranscriptionFiles::from([(
        String::from("a.wav"),
        AudioTranscriptionFileValue::Text(String::from("abc")),
    )]));
    let (request, _fixture, _config, _events, _wire) = send(data, &optional).await;
    let TranscriptionBody::Encoded(RequestBody::Multipart { boundary, parts }) = &request.body
    else {
        panic!("the files case sends multipart");
    };
    assert_eq!(boundary.len(), 32);
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0].name, "query");
    assert_eq!(parts[0].file_name, None);
    assert_eq!(parts[0].data, b"hello");
    assert_eq!(parts[1].name, "a.wav");
    assert_eq!(parts[1].file_name.as_deref(), Some("upload"));
    assert_eq!(
        parts[1].content_type.as_deref(),
        Some("application/octet-stream")
    );
    assert_eq!(parts[1].data, b"abc");
}

fn multipart_data(content_type: &str) -> AudioTranscriptionRequestData {
    let mut data = record(AudioTranscriptionData::Object(object(
        r#"{"query":"hello"}"#,
    )));
    data.files = Some(AudioTranscriptionFiles::from([(
        String::from("a.wav"),
        AudioTranscriptionFileValue::Text(String::from("abc")),
    )]));
    data.content_type = Some(content_type.to_owned());
    data
}

async fn assert_declared_boundary_is_kept() {
    let optional = object(r#"{"query":"hello"}"#);
    let (request, _fixture, _config, _events, _wire) = send(
        multipart_data("multipart/form-data; boundary=fixture-boundary"),
        &optional,
    )
    .await;
    assert_eq!(
        header(&request.headers, "content-type"),
        Some("multipart/form-data; boundary=fixture-boundary")
    );
    let bytes = body_bytes(&request.body).expect("the declared boundary sends raw bytes");
    let text = std::str::from_utf8(bytes).expect("the multipart body is text");
    assert!(text.starts_with("--fixture-boundary\r\n"));
    assert!(text.contains("name=\"query\""));
}

async fn assert_declared_type_without_boundary_gets_one() {
    let optional = object(r#"{"query":"hello"}"#);
    let (request, _fixture, _config, _events, _wire) =
        send(multipart_data("multipart/form-data"), &optional).await;
    assert_eq!(
        header(&request.headers, "content-type"),
        Some("multipart/form-data")
    );
    let bytes = body_bytes(&request.body).expect("the generated boundary sends raw bytes");
    let text = std::str::from_utf8(bytes).expect("the multipart body is text");
    let boundary = text
        .strip_prefix("--")
        .and_then(|rest| rest.split_once("\r\n"))
        .map(|(boundary, _rest)| boundary)
        .expect("the body opens with a boundary");
    assert_eq!(boundary.len(), 32);
    assert!(boundary.chars().all(|letter| letter.is_ascii_hexdigit()));
}

async fn assert_bytes_are_raw() {
    let optional = object("{}");
    let (request, _fixture, _config, _events, _wire) = send(
        record(AudioTranscriptionData::Bytes(b"abc".to_vec())),
        &optional,
    )
    .await;
    assert!(matches!(&request.body, TranscriptionBody::Raw(_)));
    assert_eq!(body_bytes(&request.body), Some(&b"abc"[..]));
    assert_eq!(header(&request.headers, "content-type"), None);
}

async fn assert_content_type_override() {
    let optional = object("{}");
    let mut custom = record(AudioTranscriptionData::Object(Object::new()));
    custom.content_type = Some(String::from("audio/custom"));
    let (custom_request, _fixture, _config, _events, _wire) = send(custom, &optional).await;
    assert_eq!(
        header(&custom_request.headers, "content-type"),
        Some("audio/custom")
    );
    let mut empty = record(AudioTranscriptionData::Object(Object::new()));
    empty.content_type = Some(String::new());
    let (empty_request, _fixture, _config, _events, _wire) = send(empty, &optional).await;
    assert_eq!(header(&empty_request.headers, "content-type"), Some(""));
}

async fn assert_validation_and_url_arguments() {
    let optional = object(r#"{"query":"hello"}"#);
    let litellm = object(r#"{"ssl_verify":false}"#);
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, r#"{"text":"hi"}"#), &events);
    let source = Source::new(wire);
    let config = RecordingConfig::new(
        record(AudioTranscriptionData::Object(object("{}"))),
        &events,
    );
    let headers = Headers::from([(String::from("X-Case"), String::from("encoding"))]);
    let mut request_call = call(&optional, &litellm);
    request_call.headers = Some(&headers);
    let mut fixture = Fixture::new("transcription");
    fixture
        .run(
            &events,
            &deps(&config, &source),
            &request_call,
            Entry::Audio,
        )
        .await
        .expect("the fixture call succeeds");
    let calls = config.calls();
    assert_eq!(calls.validations.len(), 1);
    let validation = &calls.validations[0];
    assert_eq!(validation.api_key.as_deref(), Some("sk-provider"));
    assert_eq!(validation.model, "m");
    assert_eq!(validation.messages, 0);
    assert_eq!(
        validation.headers.get("X-Case").map(String::as_str),
        Some("encoding")
    );
    assert_eq!(calls.urls.len(), 1);
    let url = &calls.urls[0];
    assert_eq!(url.api_base.as_deref(), Some("https://fixture.test/v1"));
    assert_eq!(url.api_key.as_deref(), Some("sk-provider"));
    assert_eq!(url.model, "m");
}

async fn assert_logging_input() {
    let optional = object(r#"{"query":"hello"}"#);
    let (_request, query_fixture, _config, _events, _wire) = send(
        record(AudioTranscriptionData::Object(object("{}"))),
        &optional,
    )
    .await;
    assert_eq!(
        query_fixture.input(),
        Some(&Json::Text(String::from("hello")))
    );
    let bare = object("{}");
    let (_request, bare_fixture, _config, _events, _wire) =
        send(record(AudioTranscriptionData::Object(object("{}"))), &bare).await;
    assert_eq!(bare_fixture.input(), Some(&Json::Text(String::new())));
}

async fn assert_logging_complete_input() {
    let optional = object("{}");
    let (_request, empty_fixture, _config, _events, _wire) = send(
        record(AudioTranscriptionData::Object(Object::new())),
        &optional,
    )
    .await;
    assert_eq!(empty_fixture.complete_input(), Some(&json("{}")));
    let populated = record(AudioTranscriptionData::Object(object(
        r#"{"query":"hello"}"#,
    )));
    let (_request, object_fixture, _config, _events, _wire) = send(populated, &optional).await;
    assert_eq!(
        object_fixture.complete_input(),
        Some(&json(r#"{"query":"hello"}"#))
    );
    let (_request, text_fixture, _config, _events, _wire) = send(
        record(AudioTranscriptionData::Bytes(b"abc".to_vec())),
        &optional,
    )
    .await;
    assert_eq!(
        text_fixture.complete_input(),
        Some(&Json::Text(String::from("abc")))
    );
    let (_request, bytes_fixture, _config, _events, _wire) =
        send(record(AudioTranscriptionData::Bytes(Vec::new())), &optional).await;
    assert_eq!(bytes_fixture.complete_input(), Some(&json("{}")));
}

async fn assert_logged_headers_are_masked() {
    let optional = object("{}");
    let (_request, fixture, _config, _events, _wire) = send(
        record(AudioTranscriptionData::Object(Object::new())),
        &optional,
    )
    .await;
    let headers = fixture.logged("headers").expect("the headers are logged");
    let Json::Object(headers) = headers else {
        panic!("the logged headers are an object");
    };
    assert_eq!(
        headers.get("Authorization"),
        Some(&Json::Text(String::from("Be****er")))
    );
    assert_eq!(
        fixture.logged("api_base"),
        Some(&Json::Text(String::from(
            "https://fixture.test/v1/audio/transcriptions"
        )))
    );
    assert_eq!(fixture.complete_input(), Some(&json("{}")));
}

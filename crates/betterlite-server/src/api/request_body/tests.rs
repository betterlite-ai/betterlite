#![cfg(test)]

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::request_body::Settings;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::request_body::{
    BodyKind, JsonObject, ParsedBody, RequestBodyError, RequestBodyInput, is_opaque_audio_request,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use super::{decode_request_body, prepare_request_body, read_request_body};

fn object(source: &str) -> JsonObject {
    serde_json::from_str(source).unwrap()
}

fn request(content_type: &str, route_path: &str, body: Vec<u8>) -> RequestBodyInput {
    RequestBodyInput {
        method: "POST".to_owned(),
        route_path: route_path.to_owned(),
        headers: [("content-type".to_owned(), content_type.to_owned())].into(),
        query_params: BTreeMap::new(),
        body,
        client_requested_model: None,
        cached_body: None,
        form_fields: None,
    }
}

fn settings(max_repair_bytes: Option<u64>) -> Settings {
    Settings { max_repair_bytes }
}

fn load_settings(environment: &[(&str, &str)]) -> Settings {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load("")
        .unwrap()
        .sections
        .request_body
}

#[test]
fn cached_body_single_read() {
    let cached = ParsedBody::from_client_body(object(r#"{"model":"cached","messages":[]}"#));
    let input = RequestBodyInput {
        cached_body: Some(cached),
        ..request(
            "application/json",
            "/v1/chat/completions",
            br#"{"model":"raw"}"#.to_vec(),
        )
    };
    let first = read_request_body(&input, &Settings::default(), &ApiLimits::default()).unwrap();
    assert_eq!(first.branch, BodyKind::Cached);
    assert_eq!(first.body, object(r#"{"model":"cached","messages":[]}"#));
    assert!(!first.body_read);
    assert_eq!(
        first.cached_body(),
        Some(object(r#"{"model":"cached","messages":[]}"#))
    );
    let second = read_request_body(&input, &Settings::default(), &ApiLimits::default()).unwrap();
    assert_eq!(second, first);
}

#[test]
fn repair_small_json() {
    let input = request(
        "application/json",
        "/v1/chat/completions",
        b"{\"model\":\"\xff\"}".to_vec(),
    );
    let result = read_request_body(&input, &Settings::default(), &ApiLimits::default()).unwrap();
    assert_eq!(result.branch, BodyKind::Json);
    assert_eq!(result.body, JsonObject::new());
    assert!(result.body_read);
    assert!(result.parsed_body.is_none());
    assert!(result.raw_json_body(&input).is_none());
}

#[test]
fn repair_over_threshold() {
    let input = request(
        "application/json",
        "/v1/chat/completions",
        b"{\"model\":\"\xff\"}".to_vec(),
    );
    let error = read_request_body(&input, &settings(Some(4)), &ApiLimits::default()).unwrap_err();
    assert_eq!(error, RequestBodyError::InvalidJson);
    let facts = error.facts();
    assert_eq!(facts.status, 400);
    assert_eq!(facts.error_type, "invalid_request_error");
    assert_eq!(facts.param, "request_body");
    assert_eq!(facts.code, "400");
    let uncapped = read_request_body(&input, &settings(None), &ApiLimits::default()).unwrap();
    assert_eq!(uncapped.body, JsonObject::new());
    assert!(uncapped.parsed_body.is_none());
}

#[test]
fn opaque_audio_body() {
    let input = request("audio/wav", "/azure_speech/audio/speech", b"RIFF".to_vec());
    assert!(is_opaque_audio_request(
        &input.route_path,
        input.content_type()
    ));
    let prepared =
        prepare_request_body(&input, &Settings::default(), &ApiLimits::default()).unwrap();
    assert_eq!(prepared.branch, BodyKind::OpaqueAudio);
    assert!(!prepared.body_read);
    assert_eq!(prepared.body, JsonObject::new());
    assert_eq!(prepared.cached_body(), Some(JsonObject::new()));
    assert_eq!(prepared.raw_json_body(&input), Some(&b"RIFF"[..]));
    let json = request(
        "application/json",
        "/azure_speech/audio/speech",
        br#"{"model":"gpt-4o"}"#.to_vec(),
    );
    let read = prepare_request_body(&json, &Settings::default(), &ApiLimits::default()).unwrap();
    assert_eq!(read.branch, BodyKind::Json);
}

#[test]
fn otlp_body_preserved() {
    let input = request("application/x-protobuf", "/v1/traces", vec![0x0a, 0xff]);
    let result = read_request_body(&input, &Settings::default(), &ApiLimits::default()).unwrap();
    assert_eq!(result.branch, BodyKind::Otlp);
    assert_eq!(result.body, JsonObject::new());
    assert!(!result.body_read);
    assert!(result.parsed_body.is_none());
    assert!(result.raw_json_body(&input).is_none());
    let get = RequestBodyInput {
        method: "GET".to_owned(),
        ..request(
            "application/json",
            "/v1/traces",
            br#"{"model":"gpt-4o"}"#.to_vec(),
        )
    };
    let read = read_request_body(&get, &Settings::default(), &ApiLimits::default()).unwrap();
    assert_eq!(read.branch, BodyKind::Json);
    assert!(read.body_read);
}

#[test]
fn invalid_json_body() {
    let input = request(
        "application/json",
        "/v1/chat/completions",
        br#"{"model":"#.to_vec(),
    );
    let error = read_request_body(&input, &Settings::default(), &ApiLimits::default()).unwrap_err();
    assert_eq!(error, RequestBodyError::InvalidJson);
    let facts = error.facts();
    assert_eq!(
        (facts.status, facts.error_type, facts.param, facts.code),
        (400, "invalid_request_error", "request_body", "400")
    );
}

#[test]
fn loads_settings() {
    let body = vec![0xff_u8; 1024 * 1024 + 1];
    let input = request("application/json", "/v1/chat/completions", body);
    let unset = load_settings(&[]);
    assert_eq!(unset.max_repair_bytes, Some(1024 * 1024));
    assert_eq!(
        read_request_body(&input, &unset, &ApiLimits::default()).unwrap_err(),
        RequestBodyError::InvalidJson
    );
    let disabled = load_settings(&[("MAX_REQUEST_BODY_SIZE_TO_REPAIR_MB", "0")]);
    assert_eq!(disabled.max_repair_bytes, None);
    let result = read_request_body(&input, &disabled, &ApiLimits::default()).unwrap();
    assert_eq!(result.body, JsonObject::new());
}

#[test]
fn a_non_object_json_body_is_refused() {
    let input = request(
        "application/json",
        "/v1/chat/completions",
        b"[1,2]".to_vec(),
    );
    let error = read_request_body(&input, &Settings::default(), &ApiLimits::default()).unwrap_err();
    assert_eq!(error, RequestBodyError::NotAnObject);
    assert_eq!(error.facts().status, 400);
}

#[test]
fn a_json_body_over_the_api_limit_is_refused() {
    let limits = ApiLimits {
        max_json_bytes: 8,
        max_event_bytes: 1024,
        max_items: 16,
    };
    let input = request(
        "application/json",
        "/v1/chat/completions",
        br#"{"model":"gpt-4o"}"#.to_vec(),
    );
    let error = read_request_body(&input, &Settings::default(), &limits).unwrap_err();
    let facts = error.facts();
    assert_eq!(facts.status, 413);
    assert_eq!(facts.param, "body");
    assert_eq!(facts.code, "request_too_large");
}

#[test]
fn a_binary_media_type_keeps_json_bodies() {
    let input = request(
        "application/x-protobuf",
        "/v1/embeddings",
        br#"{"input":"value"}"#.to_vec(),
    );
    let result = read_request_body(&input, &Settings::default(), &ApiLimits::default()).unwrap();
    assert_eq!(result.branch, BodyKind::Binary);
    assert_eq!(result.body, object(r#"{"input":"value"}"#));
    let real_binary = request("application/x-protobuf", "/v1/embeddings", vec![0x0a, 0xff]);
    let binary =
        read_request_body(&real_binary, &Settings::default(), &ApiLimits::default()).unwrap();
    assert_eq!(binary.cached_body(), Some(JsonObject::new()));
}

#[test]
fn a_form_body_is_lifted_with_its_metadata() {
    let input = RequestBodyInput {
        form_fields: Some(
            [
                ("model".to_owned(), "gpt-4o".to_owned()),
                ("metadata".to_owned(), r#"{"team":"a"}"#.to_owned()),
            ]
            .into(),
        ),
        ..request(
            "multipart/form-data; boundary=x",
            "/v1/audio/transcriptions",
            b"unused".to_vec(),
        )
    };
    let result = read_request_body(&input, &Settings::default(), &ApiLimits::default()).unwrap();
    assert_eq!(result.branch, BodyKind::Form);
    assert_eq!(
        result.body,
        object(r#"{"model":"gpt-4o","metadata":{"team":"a"}}"#)
    );
    assert!(result.raw_json_body(&input).is_none());
    let missing = request(
        "multipart/form-data",
        "/v1/audio/transcriptions",
        b"unused".to_vec(),
    );
    assert_eq!(
        read_request_body(&missing, &Settings::default(), &ApiLimits::default()).unwrap_err(),
        RequestBodyError::UndecodedForm
    );
}

#[test]
fn an_empty_json_body_is_cached_as_an_empty_object() {
    let input = request("application/json", "/v1/chat/completions", Vec::new());
    let result = read_request_body(&input, &Settings::default(), &ApiLimits::default()).unwrap();
    assert_eq!(result.branch, BodyKind::Json);
    assert_eq!(result.cached_body(), Some(JsonObject::new()));
}

#[test]
fn replays_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/request_body");
    let replayed = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| fs::read(entry.unwrap().path()).unwrap())
        .map(|bytes| decode_request_body(&bytes, &ApiLimits::default()).map(drop))
        .count();
    assert!(replayed >= 6);
}

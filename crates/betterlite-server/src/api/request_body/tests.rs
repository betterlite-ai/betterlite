#![cfg(test)]

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::request_body::Settings;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::request_body::{
    BodyKind, JsonObject, ParsedBody, RequestBodyError, RequestBodyInput, is_opaque_audio_request,
};
use rstest::rstest;
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::str;
use std::sync::{Arc, Mutex};
use tracing::subscriber::with_default;
use tracing_subscriber::fmt::MakeWriter;

use super::{decode_request_body, read_request_body};
use betterlite_core::api::api_boundaries::ApiLimitError;
use betterlite_core::api::request_body::DecodedJson;

#[derive(Clone)]
struct CaptureWriter(Arc<Mutex<String>>);

impl io::Write for CaptureWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let text = str::from_utf8(buf)
            .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;
        self.0
            .lock()
            .map_err(|source| io::Error::other(source.to_string()))?
            .push_str(text);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for CaptureWriter {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

fn object(source: &str) -> JsonObject {
    serde_json::from_str(source).unwrap()
}

fn request(content_type: &str, route_path: &str, body: Vec<u8>) -> RequestBodyInput {
    RequestBodyInput {
        method: "POST".to_owned(),
        route_path: route_path.to_owned(),
        headers: BTreeMap::from([("content-type".to_owned(), content_type.to_owned())]),
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
        first.parsed_body.as_ref().map(ParsedBody::visible),
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
    let records = Arc::new(Mutex::new(String::new()));
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(CaptureWriter(Arc::clone(&records)))
        .finish();
    let result = with_default(subscriber, || {
        read_request_body(&input, &Settings::default(), &ApiLimits::default()).unwrap()
    });
    assert_eq!(result.branch, BodyKind::Json);
    assert_eq!(result.body, JsonObject::new());
    assert!(result.body_read);
    assert!(result.parsed_body.is_none());
    assert!(result.raw_json_body(&input).is_none());
    let captured = records.lock().unwrap();
    assert!(captured.contains("ERROR"));
    assert!(captured.contains("/v1/chat/completions"));
    assert!(captured.contains("invalid utf-8 sequence"));
    drop(captured);
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
    assert!(is_opaque_audio_request(&input.route_path, "audio/wav"));
    let prepared = read_request_body(&input, &Settings::default(), &ApiLimits::default()).unwrap();
    assert_eq!(prepared.branch, BodyKind::OpaqueAudio);
    assert!(!prepared.body_read);
    assert_eq!(prepared.body, JsonObject::new());
    assert_eq!(
        prepared.parsed_body.as_ref().map(ParsedBody::visible),
        Some(JsonObject::new())
    );
    assert_eq!(prepared.raw_json_body(&input), Some(&b"RIFF"[..]));
    let json = request(
        "application/json",
        "/azure_speech/audio/speech",
        br#"{"model":"gpt-4o"}"#.to_vec(),
    );
    let read = read_request_body(&json, &Settings::default(), &ApiLimits::default()).unwrap();
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

#[rstest]
#[case::post_traces("POST", "/v1/traces", BodyKind::Otlp)]
#[case::get_traces("GET", "/v1/traces", BodyKind::Binary)]
#[case::other_method("PUT", "/v1/traces", BodyKind::Binary)]
#[case::trailing_slash("POST", "/v1/traces/", BodyKind::Binary)]
#[case::other_path("POST", "/v1/chat/completions", BodyKind::Binary)]
fn otlp_trace_requests_are_posted_to_the_traces_path(
    #[case] method: &str,
    #[case] route_path: &str,
    #[case] branch: BodyKind,
) {
    let input = RequestBodyInput {
        method: method.to_owned(),
        ..request("application/x-protobuf", route_path, vec![0x0a, 0xff])
    };
    let result = read_request_body(&input, &Settings::default(), &ApiLimits::default()).unwrap();
    assert_eq!(result.branch, branch);
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
    assert_eq!(
        binary.parsed_body.as_ref().map(ParsedBody::visible),
        Some(JsonObject::new())
    );
}

#[test]
fn a_binary_media_type_json_body_over_the_api_limit_is_refused() {
    let limits = ApiLimits {
        max_json_bytes: 8,
        max_event_bytes: 1024,
        max_items: 16,
    };
    let input = request(
        "application/x-protobuf",
        "/v1/embeddings",
        br#"{"input":"value"}"#.to_vec(),
    );
    let error = read_request_body(&input, &Settings::default(), &limits).unwrap_err();
    assert_eq!(error.facts().status, 413);
    let within = request("application/x-protobuf", "/v1/embeddings", b"{}".to_vec());
    let read = read_request_body(&within, &Settings::default(), &limits).unwrap();
    assert_eq!(read.branch, BodyKind::Binary);
}

#[test]
fn a_form_body_is_lifted_with_its_metadata() {
    let input = RequestBodyInput {
        form_fields: Some(BTreeMap::from([
            ("model".to_owned(), "gpt-4o".to_owned()),
            ("metadata".to_owned(), r#"{"team":"a"}"#.to_owned()),
        ])),
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
    assert_eq!(
        result.parsed_body.as_ref().map(ParsedBody::visible),
        Some(JsonObject::new())
    );
}

#[test]
fn decode_request_body_parses_an_object() {
    let decoded = decode_request_body(br#"{"model":"gpt-4o"}"#, &ApiLimits::default()).unwrap();
    assert_eq!(
        decoded,
        DecodedJson::Parsed {
            body: object(r#"{"model":"gpt-4o"}"#),
            repaired: false,
        }
    );
}

#[test]
fn decode_request_body_parses_an_empty_body_as_an_empty_object() {
    let decoded = decode_request_body(b"", &ApiLimits::default()).unwrap();
    assert_eq!(
        decoded,
        DecodedJson::Parsed {
            body: JsonObject::new(),
            repaired: false,
        }
    );
}

#[rstest]
#[case::array(b"[1,2]")]
#[case::string(br#""hi""#)]
#[case::number(b"12")]
fn decode_request_body_refuses_a_non_object(#[case] body: &[u8]) {
    assert_eq!(
        decode_request_body(body, &ApiLimits::default()),
        Err(RequestBodyError::NotAnObject)
    );
}

#[rstest]
#[case::truncated(br#"{"model":"#)]
#[case::trailing_comma(br#"{"model":"gpt-4o",}"#)]
#[case::nan(br#"{"model":"gpt-4o","x":NaN}"#)]
#[case::surrogate_escape(br#"{"x":"\ud800"}"#)]
fn decode_request_body_refuses_unrepairable_json(#[case] body: &[u8]) {
    assert_eq!(
        decode_request_body(body, &ApiLimits::default()),
        Err(RequestBodyError::InvalidJson)
    );
}

#[test]
fn decode_request_body_reports_unreadable_bytes() {
    let bytes = b"{\"model\":\"\xff\"}".to_vec();
    assert_eq!(
        decode_request_body(&bytes, &ApiLimits::default()),
        Ok(DecodedJson::Unreadable(str::from_utf8(&bytes).unwrap_err()))
    );
}

#[test]
fn decode_request_body_checks_the_json_cap_first() {
    let small = ApiLimits {
        max_json_bytes: 8,
        max_event_bytes: 1024,
        max_items: 16,
    };
    assert_eq!(
        decode_request_body(br#"{"model":"gpt-4o"}"#, &small),
        Err(RequestBodyError::Limit(ApiLimitError::JsonTooLarge {
            observed: 18,
            limit: 8,
        }))
    );
    assert_eq!(
        decode_request_body(b"[1,2]", &small),
        Err(RequestBodyError::NotAnObject)
    );
}

#[rstest]
#[case::at_the_cap(Some(13), false)]
#[case::over_the_cap(Some(12), true)]
#[case::disabled(None, false)]
fn the_repair_cap_applies_only_above_its_size(
    #[case] max_repair_bytes: Option<u64>,
    #[case] refused: bool,
) {
    let input = request(
        "application/json",
        "/v1/chat/completions",
        b"{\"model\":\"\xff\"}".to_vec(),
    );
    let outcome = read_request_body(&input, &settings(max_repair_bytes), &ApiLimits::default());
    if refused {
        assert_eq!(outcome.unwrap_err(), RequestBodyError::InvalidJson);
    } else {
        assert!(outcome.unwrap().body_read);
    }
}

#[test]
fn a_valid_object_is_never_held_to_the_repair_cap() {
    let input = request(
        "application/json",
        "/v1/chat/completions",
        br#"{"model":"gpt-4o"}"#.to_vec(),
    );
    let result = read_request_body(&input, &settings(Some(1)), &ApiLimits::default()).unwrap();
    assert_eq!(result.body, object(r#"{"model":"gpt-4o"}"#));
}

#[rstest]
fn replays_corpus(#[files("fuzz/corpus/request_body/*")] path: PathBuf) {
    let seed = fs::read(path).unwrap();
    drop(decode_request_body(&seed, &ApiLimits::default()));
}

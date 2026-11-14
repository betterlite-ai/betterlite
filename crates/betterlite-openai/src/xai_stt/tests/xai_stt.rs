use betterlite_config::sections::xai_identity::Settings;
use betterlite_http::proxy::UpstreamResponse;
use http::header::{AUTHORIZATION, HeaderMap, HeaderName, HeaderValue};
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::{Map, json};

use crate::api::audio_file_input::ProcessedAudioFile;
use crate::api::batch_records::WireDecodeLimits;
use crate::xai_stt::decode_xai_stt_reply;
use crate::xai_stt::error::XaiSttError;
use crate::xai_stt::{
    FormScalar, FormText, FormValue, SUPPORTED_OPENAI_PARAMS, authorization_headers,
    map_openai_params, stt_url, transcription_form,
};
use crate::xai_stt::{SttPayload, SttWord, TranscriptionTask};

fn settings() -> Settings {
    Settings {
        xai_key: None,
        generic_api_key: None,
        environment_key: None,
        api_base: None,
    }
}

fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

fn limits() -> WireDecodeLimits {
    WireDecodeLimits {
        max_json_bytes: 1 << 20,
        max_event_bytes: 1 << 10,
        max_items: 1000,
    }
}

fn response(status: u16, body: &[u8]) -> UpstreamResponse {
    UpstreamResponse {
        status,
        headers: HeaderMap::new(),
        body: body.to_vec(),
    }
}

fn audio() -> ProcessedAudioFile {
    ProcessedAudioFile {
        file_content: b"audio".to_vec(),
        filename: "clip.wav".to_owned(),
        content_type: "audio/wav".to_owned(),
    }
}

fn text(value: &str) -> FormValue {
    FormValue::Scalar(FormScalar::Text(value.to_owned()))
}

#[rstest]
#[case::default(None, "https://api.x.ai/v1/stt")]
#[case::trailing_slash(Some("https://proxy.example/v1/"), "https://proxy.example/v1/stt")]
#[case::versioned(Some("https://proxy.example/v1"), "https://proxy.example/v1/stt")]
#[case::unversioned(Some("https://proxy.example"), "https://proxy.example/v1/stt")]
#[case::other_version(Some("https://proxy.example/v2"), "https://proxy.example/v2/v1/stt")]
#[case::many_slashes(Some("https://proxy.example///"), "https://proxy.example/v1/stt")]
fn the_stt_url_strips_a_trailing_slash_and_one_version(
    #[case] api_base: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(stt_url(api_base, &settings()), expected);
}

#[test]
fn the_settings_api_base_is_used_when_the_call_has_none() {
    let settings = Settings {
        api_base: Some("https://settings.example/v1".to_owned()),
        ..settings()
    };
    assert_eq!(stt_url(None, &settings), "https://settings.example/v1/stt");
}

#[test]
fn the_explicit_key_wins_over_the_module_key_and_the_environment() {
    let settings = Settings {
        xai_key: Some(secret("module")),
        environment_key: Some(secret("environment")),
        ..settings()
    };
    let headers =
        authorization_headers(HeaderMap::new(), Some(&secret("explicit")), &settings).unwrap();
    assert_eq!(
        headers
            .get(AUTHORIZATION)
            .map(|value| value.to_str().unwrap()),
        Some("Bearer explicit")
    );
}

#[test]
fn the_module_key_wins_over_the_environment() {
    let settings = Settings {
        xai_key: Some(secret("module")),
        environment_key: Some(secret("environment")),
        ..settings()
    };
    let headers = authorization_headers(HeaderMap::new(), None, &settings).unwrap();
    assert_eq!(
        headers
            .get(AUTHORIZATION)
            .map(|value| value.to_str().unwrap()),
        Some("Bearer module")
    );
}

#[test]
fn the_generic_api_key_is_not_a_fallback() {
    let settings = Settings {
        generic_api_key: Some(secret("generic")),
        ..settings()
    };
    assert!(matches!(
        authorization_headers(HeaderMap::new(), None, &settings),
        Err(XaiSttError::MissingApiKey)
    ));
}

#[test]
fn a_missing_key_is_a_local_error_without_a_status() {
    let error = authorization_headers(HeaderMap::new(), None, &settings()).unwrap_err();
    assert_eq!(error.status_code(), None);
    assert_eq!(
        error.to_string(),
        "xAI API key is required. Set XAI_API_KEY environment variable."
    );
}

#[test]
fn the_caller_headers_survive_the_authorization_header() {
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("x-caller"),
        HeaderValue::from_static("kept"),
    );
    let resolved = authorization_headers(headers, Some(&secret("k")), &settings()).unwrap();
    assert_eq!(
        resolved
            .get("x-caller")
            .map(|value| value.to_str().unwrap()),
        Some("kept")
    );
    assert!(resolved.contains_key(AUTHORIZATION));
}

#[test]
fn only_language_is_advertised() {
    assert_eq!(SUPPORTED_OPENAI_PARAMS, ["language"]);
}

#[test]
fn the_option_mapper_adds_only_the_advertised_language() {
    let mut optional = Map::from_iter([("response_format".to_owned(), json!("json"))]);
    let non_default = vec![
        ("language".to_owned(), json!("en")),
        ("prompt".to_owned(), json!("direct")),
    ];
    map_openai_params(&non_default, &mut optional);
    assert_eq!(
        optional,
        Map::from_iter([
            ("language".to_owned(), json!("en")),
            ("response_format".to_owned(), json!("json")),
        ])
    );
}

#[test]
fn a_direct_language_replaces_the_optional_one() {
    let mut optional = Map::from_iter([("language".to_owned(), json!("fr"))]);
    map_openai_params(&[("language".to_owned(), json!("en"))], &mut optional);
    assert_eq!(optional.get("language"), Some(&json!("en")));
}

#[test]
fn the_form_starts_with_the_model_and_the_file() {
    let form = transcription_form("grok-stt", &audio(), &IndexMap::new(), &IndexMap::new());
    assert_eq!(form.fields.len(), 1);
    assert_eq!(form.fields[0].0, "model");
    assert_eq!(
        form.fields[0].1.form_text(),
        FormText::Scalar("grok-stt".to_owned())
    );
    assert_eq!(form.file.name, "file");
    assert_eq!(form.file.file_name.as_deref(), Some("clip.wav"));
    assert_eq!(form.file.content_type.as_deref(), Some("audio/wav"));
    assert_eq!(form.file.data, b"audio");
}

#[test]
fn extra_body_is_flattened_before_the_direct_options() {
    let extra = IndexMap::from([
        ("language".to_owned(), text("fr")),
        (
            "diarize".to_owned(),
            FormValue::Scalar(FormScalar::Bool(true)),
        ),
        ("custom".to_owned(), text("x")),
    ]);
    let direct = IndexMap::from([
        ("language".to_owned(), text("en")),
        ("prompt".to_owned(), text("hi")),
    ]);
    let form = transcription_form("m", &audio(), &extra, &direct);
    let names: Vec<&str> = form.fields.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["model", "language", "diarize", "custom", "prompt"]);
    assert_eq!(
        form.fields[1].1.form_text(),
        FormText::Scalar("en".to_owned())
    );
}

#[test]
fn a_direct_none_drops_the_extra_body_value() {
    let extra = IndexMap::from([
        ("prompt".to_owned(), text("x")),
        ("n".to_owned(), text("1")),
    ]);
    let direct = IndexMap::from([("prompt".to_owned(), FormValue::Scalar(FormScalar::Null))]);
    let form = transcription_form("m", &audio(), &extra, &direct);
    let names: Vec<&str> = form.fields.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["model", "n"]);
}

#[test]
fn excluded_names_never_reach_the_form() {
    let extra = IndexMap::from([
        ("model".to_owned(), text("nested")),
        ("OPENAI_TRANSCRIPTION_PARAMS".to_owned(), text("nested")),
        ("extra_body".to_owned(), text("nested")),
        ("language".to_owned(), text("en")),
    ]);
    let direct = IndexMap::from([("model".to_owned(), text("direct"))]);
    let form = transcription_form("grok-stt", &audio(), &extra, &direct);
    let names: Vec<&str> = form.fields.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["model", "language"]);
    assert_eq!(
        form.fields[0].1.form_text(),
        FormText::Scalar("grok-stt".to_owned())
    );
}

#[rstest]
#[case::scalar_true(FormValue::Scalar(FormScalar::Bool(true)), FormText::Scalar("true".to_owned()))]
#[case::scalar_false(FormValue::Scalar(FormScalar::Bool(false)), FormText::Scalar("false".to_owned()))]
#[case::integer(FormValue::Scalar(FormScalar::Integer(3)), FormText::Scalar("3".to_owned()))]
#[case::float(FormValue::Scalar(FormScalar::Float(0.5)), FormText::Scalar("0.5".to_owned()))]
#[case::whole_float(FormValue::Scalar(FormScalar::Float(1.0)), FormText::Scalar("1.0".to_owned()))]
#[case::small_float(FormValue::Scalar(FormScalar::Float(0.00001)), FormText::Scalar("1e-5".to_owned()))]
#[case::large_float(FormValue::Scalar(FormScalar::Float(1e16)), FormText::Scalar("1e16".to_owned()))]
#[case::list_bools(
    FormValue::List(vec![FormScalar::Bool(true), FormScalar::Bool(false), FormScalar::Integer(1)]),
    FormText::Repeated(vec!["True".to_owned(), "False".to_owned(), "1".to_owned()])
)]
#[case::list_null(
    FormValue::List(vec![FormScalar::Null]),
    FormText::Repeated(vec!["None".to_owned()])
)]
fn a_form_value_uses_pythons_string_spelling(#[case] value: FormValue, #[case] expected: FormText) {
    assert_eq!(value.form_text(), expected);
}

#[test]
fn a_key_with_a_control_character_cannot_become_a_header() {
    let error = authorization_headers(HeaderMap::new(), Some(&secret("bad\nkey")), &settings())
        .unwrap_err();
    assert_eq!(error.status_code(), None);
    assert!(matches!(error, XaiSttError::Authorization { .. }));
}

#[test]
fn a_form_value_reads_from_json_in_python_order() {
    let value: FormValue = serde_json::from_value(json!([true, 2_i32, 1.5_f64, "x"])).unwrap();
    assert_eq!(
        value.form_text(),
        FormText::Repeated(vec![
            "True".to_owned(),
            "2".to_owned(),
            "1.5".to_owned(),
            "x".to_owned()
        ])
    );
    let scalar: FormValue = serde_json::from_value(json!(false)).unwrap();
    assert_eq!(scalar.form_text(), FormText::Scalar("false".to_owned()));
}

#[test]
fn an_empty_payload_takes_the_defaults() {
    let result = decode_xai_stt_reply(&response(200, b"{}"), &limits()).unwrap();
    assert_eq!(result.text, "");
    assert_eq!(result.task, TranscriptionTask::Transcribe);
    assert_eq!(result.language, "unknown");
    assert_eq!(result.duration, None);
    assert_eq!(result.words, None);
    assert_eq!(result.metadata.native.duration, None);
    assert_eq!(result.metadata.native.words, None);
    assert_eq!(result.metadata.audio_transcription_duration, None);
}

#[test]
fn words_are_renamed_and_a_zero_speaker_is_kept() {
    let body = br#"{"text":"hello","language":"en","duration":2.5,"words":[{"text":"he","start":0.0,"end":1.0,"speaker":0},{"text":"llo","start":1.0,"end":2.0}]}"#;
    let result = decode_xai_stt_reply(&response(200, body), &limits()).unwrap();
    let words = result.words.unwrap();
    assert_eq!(words.len(), 2);
    assert_eq!(words[0].word, "he");
    assert_eq!(words[0].start.to_bits(), 0.0_f64.to_bits());
    assert_eq!(words[0].end.to_bits(), 1.0_f64.to_bits());
    assert_eq!(words[0].speaker, Some(0));
    assert_eq!(words[1].word, "llo");
    assert_eq!(words[1].speaker, None);
    assert_eq!(result.duration.map(f64::to_bits), Some(2.5_f64.to_bits()));
}

#[test]
fn the_native_payload_is_kept_with_the_duration_copied_out() {
    let body = br#"{"text":"hello","language":"en","duration":2.5,"words":[{"text":"he","start":0.0,"end":1.0,"speaker":0}],"speaker_count":2}"#;
    let result = decode_xai_stt_reply(&response(200, body), &limits()).unwrap();
    assert_eq!(result.metadata.native.text, "hello");
    assert_eq!(result.metadata.native.language, "en");
    assert_eq!(
        result.metadata.native.duration.map(f64::to_bits),
        Some(2.5_f64.to_bits())
    );
    assert_eq!(
        result
            .metadata
            .audio_transcription_duration
            .map(f64::to_bits),
        Some(2.5_f64.to_bits())
    );
    let words = result.metadata.native.words.unwrap();
    assert_eq!(words[0].text, "he");
    assert_eq!(words[0].speaker, Some(0));
    assert_eq!(
        result.metadata.native.native_fields.get("speaker_count"),
        Some(&json!(2_i32))
    );
}

#[test]
fn extra_word_fields_ride_along_in_the_native_payload() {
    let body = br#"{"words":[{"text":"x","confidence":0.9}]}"#;
    let result = decode_xai_stt_reply(&response(200, body), &limits()).unwrap();
    let word: &SttWord = &result.metadata.native.words.unwrap()[0];
    assert_eq!(word.native_fields.get("confidence"), Some(&json!(0.9_f64)));
    assert_eq!(result.words.unwrap()[0].word, "x");
}

#[test]
fn an_empty_word_list_stays_an_empty_list() {
    let result = decode_xai_stt_reply(&response(200, br#"{"words":[]}"#), &limits()).unwrap();
    assert_eq!(result.words, Some(Vec::new()));
    assert_eq!(result.metadata.native.words, Some(Vec::new()));
}

#[rstest]
#[case::text_of_the_wrong_type(br#"{"text":5}"#)]
#[case::words_of_the_wrong_type(br#"{"words":{}}"#)]
#[case::duration_of_the_wrong_type(br#"{"duration":"2.5"}"#)]
#[case::start_of_the_wrong_type(br#"{"words":[{"start":"0"}]}"#)]
#[case::json_array(b"[]")]
#[case::not_json(b"not json")]
fn a_malformed_success_keeps_the_status_it_was_handed(#[case] body: &[u8]) {
    let error = decode_xai_stt_reply(&response(201, body), &limits()).unwrap_err();
    assert_eq!(error.status_code(), Some(201));
}

#[test]
fn a_non_2xx_reply_keeps_the_status_and_headers() {
    let mut reply_headers = HeaderMap::new();
    reply_headers.insert(
        HeaderName::from_static("x-request-id"),
        HeaderValue::from_static("abc"),
    );
    let reply = UpstreamResponse {
        status: 422,
        headers: reply_headers,
        body: b"bad audio".to_vec(),
    };
    let error = decode_xai_stt_reply(&reply, &limits()).unwrap_err();
    assert_eq!(error.status_code(), Some(422));
    match error {
        XaiSttError::Status {
            status, headers, ..
        } => {
            assert_eq!(status, 422);
            assert_eq!(headers.get("x-request-id").map(String::as_str), Some("abc"));
        }
        other @ (XaiSttError::MissingApiKey
        | XaiSttError::Authorization { .. }
        | XaiSttError::Decode { .. }
        | XaiSttError::TooLarge { .. }) => {
            panic!("expected a status error, got {other:?}")
        }
    }
}

#[test]
fn a_non_utf8_error_body_still_raises_a_status_error() {
    let error = decode_xai_stt_reply(&response(500, &[0xff, 0xfe]), &limits()).unwrap_err();
    assert_eq!(error.status_code(), Some(500));
    assert!(matches!(error, XaiSttError::Status { .. }));
}

#[test]
fn a_malformed_success_keeps_the_headers_too() {
    let mut reply_headers = HeaderMap::new();
    reply_headers.insert(
        HeaderName::from_static("x-request-id"),
        HeaderValue::from_static("abc"),
    );
    let reply = UpstreamResponse {
        status: 200,
        headers: reply_headers,
        body: b"{".to_vec(),
    };
    let error = decode_xai_stt_reply(&reply, &limits()).unwrap_err();
    match error {
        XaiSttError::Decode {
            status, headers, ..
        } => {
            assert_eq!(status, 200);
            assert_eq!(headers.get("x-request-id").map(String::as_str), Some("abc"));
        }
        other @ (XaiSttError::MissingApiKey
        | XaiSttError::Authorization { .. }
        | XaiSttError::Status { .. }
        | XaiSttError::TooLarge { .. }) => {
            panic!("expected a decode error, got {other:?}")
        }
    }
}

#[test]
fn an_answer_over_the_byte_limit_keeps_the_status() {
    let limits = WireDecodeLimits {
        max_json_bytes: 4,
        max_event_bytes: 1,
        max_items: 1,
    };
    let error = decode_xai_stt_reply(&response(200, br#"{"text":"hello"}"#), &limits).unwrap_err();
    assert_eq!(error.status_code(), Some(200));
    assert!(matches!(error, XaiSttError::TooLarge { limit: 4, .. }));
}

#[test]
fn a_reply_exactly_at_the_byte_limit_is_decoded() {
    let limits = WireDecodeLimits {
        max_json_bytes: 2,
        max_event_bytes: 1,
        max_items: 1,
    };
    let result = decode_xai_stt_reply(&response(200, b"{}"), &limits).unwrap();
    assert_eq!(result.text, "");
    assert_eq!(result.language, "unknown");
}

#[test]
fn the_payload_type_reads_an_int_duration_as_a_float() {
    let payload: SttPayload = serde_json::from_slice(br#"{"duration":2}"#).unwrap();
    assert_eq!(payload.duration, Some(2.0_f64));
}

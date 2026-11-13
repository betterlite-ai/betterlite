#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use http::header::{HeaderMap, HeaderName, HeaderValue};
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::{Value, json};

use super::error::MinimaxSpeechError;
use super::params::{DEFAULT_VOICE_ID, MappedOptions, VoiceInput, map_openai_params};
use super::reply::{SpeechReply, decode_speech_reply};
use super::request::{get_complete_url, speech_request_body, validate_environment};
use crate::openai_base_config::{Headers, Object};

fn object(value: Value) -> Object {
    let Value::Object(fields) = value else {
        panic!("the fixture must be a JSON object");
    };
    fields
}

fn mapped(voice: Option<VoiceInput<'_>>, params: Value) -> MappedOptions {
    map_openai_params(voice, &object(params)).expect("the options are valid")
}

#[derive(Debug)]
struct Decoded {
    status: u16,
    headers: IndexMap<String, String>,
    content_type: Option<String>,
    body: Vec<u8>,
}

async fn decode(
    body: &str,
    status: u16,
    headers: &HeaderMap,
) -> Result<Decoded, MinimaxSpeechError> {
    let response = decode_speech_reply(SpeechReply {
        status,
        headers,
        body: body.as_bytes(),
    })?;
    let decoded = Decoded {
        status: response.status,
        content_type: response.content_type.clone(),
        headers: response.headers.clone(),
        body: response
            .content(1 << 20)
            .await
            .expect("the audio is one chunk"),
    };
    Ok(decoded)
}

fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
    let mut found = HeaderMap::new();
    for (name, value) in pairs {
        found.insert(
            HeaderName::from_static(name),
            HeaderValue::from_static(value),
        );
    }
    found
}

#[rstest]
#[case::alloy("alloy", "male-qn-qingse")]
#[case::echo("echo", "male-qn-jingying")]
#[case::fable("fable", "female-shaonv")]
#[case::onyx("onyx", "male-qn-badao")]
#[case::nova("nova", "female-yujie")]
#[case::shimmer("shimmer", "female-tianmei")]
#[case::unknown("custom-voice", "custom-voice")]
#[case::uppercase("ALLOY", "male-qn-qingse")]
#[case::padded("  echo  ", "male-qn-jingying")]
#[case::empty("", "")]
fn maps_the_voice_name(#[case] voice: &str, #[case] expected: &str) {
    let result = mapped(Some(VoiceInput::Text(voice)), json!({}));
    assert_eq!(result.voice, expected);
    assert_eq!(result.params["voice_id"], Value::from(expected));
}

#[test]
fn a_missing_voice_is_the_default() {
    let result = mapped(None, json!({}));
    assert_eq!(result.voice, DEFAULT_VOICE_ID);
    assert_eq!(result.params["voice_id"], Value::from(DEFAULT_VOICE_ID));
}

#[test]
fn the_voice_id_option_names_the_voice_when_voice_is_missing() {
    let result = mapped(None, json!({"voice_id": "nova"}));
    assert_eq!(result.voice, "female-yujie");
    assert_eq!(result.params["voice_id"], "female-yujie");
}

#[test]
fn an_empty_voice_id_option_falls_back_to_the_default() {
    let result = mapped(None, json!({"voice_id": "  "}));
    assert_eq!(result.voice, DEFAULT_VOICE_ID);
}

#[test]
fn a_voice_object_names_the_voice_from_its_first_string_key() {
    let fields = object(json!({"voice_id": "alloy", "id": "nova"}));
    let result = map_openai_params(Some(VoiceInput::Fields(&fields)), &Object::new())
        .expect("the voice object is valid");
    assert_eq!(result.voice, "male-qn-qingse");
}

#[test]
fn a_voice_object_uses_id_then_name() {
    let fields = object(json!({"id": "nova"}));
    let by_id = map_openai_params(Some(VoiceInput::Fields(&fields)), &Object::new())
        .expect("the voice object is valid");
    assert_eq!(by_id.voice, "female-yujie");
    let fields = object(json!({"name": "shimmer"}));
    let by_name = map_openai_params(Some(VoiceInput::Fields(&fields)), &Object::new())
        .expect("the voice object is valid");
    assert_eq!(by_name.voice, "female-tianmei");
}

#[test]
fn a_voice_object_without_a_usable_name_takes_the_default() {
    let fields = object(json!({"voice_id": 5, "id": "   "}));
    let result = map_openai_params(Some(VoiceInput::Fields(&fields)), &Object::new())
        .expect("the voice object is valid");
    assert_eq!(result.voice, DEFAULT_VOICE_ID);
}

#[test]
fn the_voice_argument_wins_only_until_the_voice_id_option_is_passed_through() {
    let result = mapped(
        Some(VoiceInput::Text("alloy")),
        json!({"voice_id": "custom"}),
    );
    assert_eq!(result.params["voice_id"], "custom");
}

#[rstest]
#[case::mp3("mp3", "mp3")]
#[case::pcm("pcm", "pcm")]
#[case::wav("wav", "wav")]
#[case::flac("flac", "flac")]
#[case::unknown("opus", "mp3")]
#[case::missing(None, "mp3")]
#[case::null(Some(Value::Null), "mp3")]
fn maps_the_response_format(#[case] format: Option<Value>, #[case] expected: &str) {
    let params = match format {
        Some(value) => object(json!({"response_format": value})),
        None => Object::new(),
    };
    let result = map_openai_params(None, &params).expect("the format is valid");
    assert_eq!(result.params["format"], expected);
}

#[test]
fn rejects_a_response_format_that_is_not_a_string() {
    let error = map_openai_params(None, &object(json!({"response_format": 4})))
        .expect_err("a number is not a response_format");
    assert_eq!(error.status_code(), Some(400));
}

#[rstest]
#[case::below(json!(0.2), 0.5)]
#[case::at_minimum(json!(0.5), 0.5)]
#[case::inside(json!(1.25), 1.25)]
#[case::at_maximum(json!(2), 2.0)]
#[case::above(json!(3), 2.0)]
#[case::missing(None, 1.0)]
fn clamps_the_speed(#[case] speed: Option<Value>, #[case] expected: f64) {
    let params = match speed {
        Some(value) => object(json!({"speed": value})),
        None => Object::new(),
    };
    let result = map_openai_params(None, &params).expect("the speed is valid");
    assert_eq!(result.params["speed"], Value::from(expected));
}

#[test]
fn rejects_a_speed_that_is_not_a_number() {
    let error = map_openai_params(None, &object(json!({"speed": "fast"})))
        .expect_err("a string is not a speed");
    assert_eq!(error.status_code(), Some(400));
}

#[test]
fn drops_instructions_and_passes_other_options_through() {
    let result = mapped(
        None,
        json!({"instructions": "speak slowly", "language_boost": "auto", "empty": null}),
    );
    assert!(!result.params.contains_key("instructions"));
    assert_eq!(result.params["language_boost"], "auto");
    assert!(!result.params.contains_key("empty"));
}

#[test]
fn extra_body_options_overwrite_the_mapped_ones() {
    let result = mapped(
        None,
        json!({
            "extra_body": {"language_boost": "auto", "speed": 2, "ignored": null},
            "speed": 0.75
        }),
    );
    assert_eq!(result.params["language_boost"], "auto");
    assert_eq!(result.params["speed"], 2);
    assert!(!result.params.contains_key("ignored"));
}

#[rstest]
#[case::default(None, None, "https://api.minimax.io/v1/t2a_v2")]
#[case::environment(None, Some("https://env.example"), "https://env.example/v1/t2a_v2")]
#[case::api_base(
    Some("https://base.example"),
    Some("https://env.example"),
    "https://base.example/v1/t2a_v2"
)]
#[case::trailing_slash(Some("https://base.example/"), None, "https://base.example/v1/t2a_v2")]
fn builds_the_url(
    #[case] api_base: Option<&str>,
    #[case] environment_base: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(get_complete_url(api_base, environment_base), expected);
}

#[test]
fn the_api_key_wins_over_the_global_and_environment_keys() {
    let key = SecretString::from("direct");
    let global = SecretString::from("global");
    let environment = SecretString::from("environment");
    let headers = validate_environment(
        &Headers::new(),
        Some(&key),
        Some(&global),
        Some(&environment),
    )
    .expect("a key is set");
    assert_eq!(headers["Authorization"], "Bearer direct");
    assert_eq!(headers["Content-Type"], "application/json");
}

#[test]
fn the_global_key_wins_over_the_environment_key() {
    let global = SecretString::from("global");
    let environment = SecretString::from("environment");
    let headers = validate_environment(&Headers::new(), None, Some(&global), Some(&environment))
        .expect("a key is set");
    assert_eq!(headers["Authorization"], "Bearer global");
}

#[test]
fn the_environment_key_is_the_last_source() {
    let environment = SecretString::from("environment");
    let headers = validate_environment(&Headers::new(), None, None, Some(&environment))
        .expect("a key is set");
    assert_eq!(headers["Authorization"], "Bearer environment");
}

#[test]
fn a_missing_key_names_the_environment_variable() {
    let error = validate_environment(&Headers::new(), None, None, None).expect_err("no key is set");
    assert_eq!(error.status_code(), None);
    assert!(error.to_string().contains("MINIMAX_API_KEY"));
}

#[test]
fn keeps_the_callers_headers_beside_the_two_it_sets() {
    let key = SecretString::from("direct");
    let mut given = Headers::new();
    given.insert(String::from("X-Trace"), String::from("abc"));
    let headers = validate_environment(&given, Some(&key), None, None).expect("a key is set");
    assert_eq!(headers["X-Trace"], "abc");
    assert_eq!(headers.len(), 3);
}

#[test]
fn builds_the_default_body() {
    let body = speech_request_body("speech-01", "hello", "male-qn-qingse", &Object::new());
    assert_eq!(
        Value::Object(body),
        json!({
            "model": "speech-01",
            "text": "hello",
            "stream": false,
            "output_format": "hex",
            "voice_setting": {"voice_id": "male-qn-qingse", "speed": 1.0, "vol": 1.0, "pitch": 0},
            "audio_setting": {"sample_rate": 32000, "bitrate": 128000, "format": "mp3", "channel": 1}
        })
    );
}

#[test]
fn the_body_takes_the_mapped_options() {
    let options = mapped(
        Some(VoiceInput::Text("shimmer")),
        json!({"response_format": "wav", "speed": 1.5}),
    );
    let body = speech_request_body("speech-01", "hello", &options.voice, &options.params);
    assert_eq!(body["voice_setting"]["voice_id"], "female-tianmei");
    assert_eq!(body["voice_setting"]["speed"], 1.5);
    assert_eq!(body["audio_setting"]["format"], "wav");
}

#[test]
fn the_body_takes_audio_and_voice_settings_from_the_options() {
    let options = object(json!({
        "vol": 3.0,
        "pitch": -4,
        "sample_rate": 16000,
        "bitrate": 64000,
        "channel": 2,
        "format": "flac",
        "output_format": "url"
    }));
    let body = speech_request_body("speech-01", "hello", "nova", &options);
    assert_eq!(body["voice_setting"]["vol"], 3.0);
    assert_eq!(body["voice_setting"]["pitch"], -4);
    assert_eq!(body["audio_setting"]["sample_rate"], 16000);
    assert_eq!(body["audio_setting"]["bitrate"], 64000);
    assert_eq!(body["audio_setting"]["channel"], 2);
    assert_eq!(body["audio_setting"]["format"], "flac");
    assert_eq!(body["output_format"], "url");
}

#[test]
fn an_absent_or_null_voice_setting_takes_the_default() {
    let options = object(json!({"voice_id": null, "speed": null, "format": null}));
    let body = speech_request_body("speech-01", "hello", "", &options);
    assert_eq!(body["voice_setting"]["voice_id"], DEFAULT_VOICE_ID);
    assert_eq!(body["voice_setting"]["speed"], 1.0);
    assert_eq!(body["audio_setting"]["format"], "mp3");
}

#[test]
fn extra_body_keys_join_the_body_only_when_it_lacks_them() {
    let options =
        object(json!({"extra_body": {"language_boost": "auto", "text": "replaced", "drop": null}}));
    let body = speech_request_body("speech-01", "hello", "nova", &options);
    assert_eq!(body["language_boost"], "auto");
    assert_eq!(body["text"], "hello");
    assert!(!body.contains_key("drop"));
}

#[tokio::test]
async fn decodes_hex_audio_and_cleans_the_headers() {
    let headers = headers(&[
        ("content-type", "audio/mpeg"),
        ("content-encoding", "gzip"),
        ("transfer-encoding", "chunked"),
        ("content-length", "999"),
    ]);
    let decoded = decode(
        r#"{"status": 0, "data": {"audio": "48656c6c6f"}}"#,
        200,
        &headers,
    )
    .await
    .expect("the reply carries hex audio");
    assert_eq!(decoded.status, 200);
    assert_eq!(decoded.body, b"Hello".to_vec());
    assert_eq!(decoded.content_type.as_deref(), Some("audio/mpeg"));
    assert_eq!(decoded.headers["content-length"], "5");
    assert!(!decoded.headers.contains_key("content-encoding"));
    assert!(!decoded.headers.contains_key("transfer-encoding"));
}

#[tokio::test]
async fn decodes_base64_when_the_audio_is_not_hex() {
    let headers = HeaderMap::new();
    let decoded = decode(r#"{"data": {"audio": "SGVsbG8="}}"#, 200, &headers)
        .await
        .expect("the reply carries base64 audio");
    assert_eq!(decoded.body, b"Hello".to_vec());
}

#[tokio::test]
async fn hex_audio_may_carry_whitespace() {
    let headers = HeaderMap::new();
    let decoded = decode(r#"{"data": {"audio": "48 65 6c\n6c6f"}}"#, 200, &headers)
        .await
        .expect("whitespace inside hex is ignored");
    assert_eq!(decoded.body, b"Hello".to_vec());
}

#[tokio::test]
async fn a_non_zero_status_carries_the_raw_status_and_the_ced_text() {
    let headers = HeaderMap::new();
    let error = decode(r#"{"status": 2, "ced": "invalid api key"}"#, 401, &headers)
        .await
        .expect_err("status 2 is an error");
    assert_eq!(error.status_code(), Some(401));
    assert!(
        error
            .to_string()
            .contains("MiniMax TTS error: invalid api key")
    );
}

#[tokio::test]
async fn an_empty_ced_names_the_status() {
    let headers = HeaderMap::new();
    let error = decode(r#"{"status": 2, "ced": ""}"#, 400, &headers)
        .await
        .expect_err("status 2 is an error");
    assert!(error.to_string().contains("API returned status 2"));
}

#[tokio::test]
async fn a_null_ced_names_the_status() {
    let headers = HeaderMap::new();
    let error = decode(r#"{"status": 3, "ced": null}"#, 400, &headers)
        .await
        .expect_err("status 3 is an error");
    assert!(error.to_string().contains("API returned status 3"));
}

#[tokio::test]
async fn a_ced_of_the_wrong_type_is_an_error() {
    let headers = HeaderMap::new();
    let error = decode(r#"{"status": 3, "ced": 7}"#, 400, &headers)
        .await
        .expect_err("a numeric ced cannot be used");
    assert_eq!(error.status_code(), Some(500));
}

#[tokio::test]
async fn a_missing_ced_is_unknown() {
    let headers = HeaderMap::new();
    let error = decode(r#"{"status": 2}"#, 400, &headers)
        .await
        .expect_err("status 2 is an error");
    assert!(error.to_string().contains("Unknown error"));
}

#[tokio::test]
async fn a_zero_or_absent_status_is_success() {
    let headers = HeaderMap::new();
    let zero = decode(r#"{"status": 0, "data": {"audio": "4869"}}"#, 200, &headers)
        .await
        .expect("status 0 is success");
    assert_eq!(zero.body, b"Hi".to_vec());
    let absent = decode(r#"{"data": {"audio": "4869"}}"#, 200, &headers)
        .await
        .expect("an absent status is success");
    assert_eq!(absent.body, b"Hi".to_vec());
}

#[tokio::test]
async fn an_audio_url_is_not_supported() {
    let headers = HeaderMap::new();
    let error = decode(
        r#"{"data": {"audio_url": "https://example/audio.mp3"}}"#,
        200,
        &headers,
    )
    .await
    .expect_err("a URL reply is unsupported");
    assert_eq!(error.status_code(), Some(500));
    assert!(
        error
            .to_string()
            .contains("URL output format is not yet supported")
    );
}

#[tokio::test]
async fn an_empty_audio_url_is_no_url() {
    let headers = HeaderMap::new();
    let error = decode(r#"{"data": {"audio_url": ""}}"#, 200, &headers)
        .await
        .expect_err("there is no audio");
    assert!(error.to_string().contains("No audio data"));
}

#[tokio::test]
async fn a_reply_without_audio_is_an_error() {
    let headers = HeaderMap::new();
    let error = decode(r#"{"status": 0}"#, 200, &headers)
        .await
        .expect_err("there is no audio");
    assert_eq!(error.status_code(), Some(500));
    assert!(error.to_string().contains("No audio data"));
}

#[tokio::test]
async fn the_top_level_audio_file_is_the_fallback() {
    let headers = HeaderMap::new();
    let decoded = decode(r#"{"audio_file": "48656c6c6f"}"#, 200, &headers)
        .await
        .expect("the older reply carries top level audio");
    assert_eq!(decoded.body, b"Hello".to_vec());
}

#[tokio::test]
async fn data_audio_wins_over_the_top_level_audio_file() {
    let headers = HeaderMap::new();
    let decoded = decode(
        r#"{"data": {"audio": "4869"}, "audio_file": "427965"}"#,
        200,
        &headers,
    )
    .await
    .expect("data.audio comes first");
    assert_eq!(decoded.body, b"Hi".to_vec());
}

#[tokio::test]
async fn data_audio_file_is_never_read() {
    let headers = HeaderMap::new();
    let error = decode(r#"{"data": {"audio_file": "4869"}}"#, 200, &headers)
        .await
        .expect_err("only the top level audio_file is a fallback");
    assert!(error.to_string().contains("No audio data"));
}

#[tokio::test]
async fn audio_that_is_neither_hex_nor_base64_is_an_error() {
    let headers = HeaderMap::new();
    let error = decode(r#"{"data": {"audio": "abc"}}"#, 200, &headers)
        .await
        .expect_err("the audio cannot be decoded");
    assert_eq!(error.status_code(), Some(500));
}

#[tokio::test]
async fn a_reply_that_is_not_json_is_an_error() {
    let headers = HeaderMap::new();
    let error = decode("{not json", 200, &headers)
        .await
        .expect_err("the reply is not JSON");
    assert_eq!(error.status_code(), Some(500));
}

#[tokio::test]
async fn a_reply_that_is_not_an_object_is_an_error() {
    let headers = HeaderMap::new();
    let error = decode("[1, 2]", 200, &headers)
        .await
        .expect_err("the reply is not an object");
    assert_eq!(error.status_code(), Some(500));
}

#[tokio::test]
async fn a_wrong_typed_audio_field_is_an_error() {
    let headers = HeaderMap::new();
    let error = decode(r#"{"data": {"audio": 5}}"#, 200, &headers)
        .await
        .expect_err("a number is not audio");
    assert_eq!(error.status_code(), Some(500));
}

#[tokio::test]
async fn an_empty_data_audio_is_used_as_given() {
    let headers = HeaderMap::new();
    let error = decode(
        r#"{"data": {"audio": ""}, "audio_file": "4869"}"#,
        200,
        &headers,
    )
    .await
    .expect_err("an explicit empty audio is not a fallback");
    assert!(error.to_string().contains("No audio data"));
}

#[test]
fn replays_the_minimax_speech_reply_corpus() {
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/minimax_speech_reply");
    let mut paths: Vec<PathBuf> = fs::read_dir(corpus)
        .expect("the corpus folder exists")
        .map(|entry| entry.expect("the corpus entry reads").path())
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no minimax speech corpus seeds");
    let headers = HeaderMap::new();
    paths.into_iter().for_each(|path| {
        let bytes = fs::read(&path).expect("the corpus seed reads");
        let _decoded = decode_speech_reply(SpeechReply {
            status: 200,
            headers: &headers,
            body: &bytes,
        });
    });
}

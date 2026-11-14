#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::decisions::Json;
use betterlite_openai::api::audio_file_input::ProcessedAudioFile;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::reasoning_models::DropParams;
use http::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use rstest::rstest;
use secrecy::SecretString;

use crate::gemini_response::decode::MAX_RESPONSE_BYTES;
use crate::vertex_gemini_transcription::error::VertexGeminiTranscriptionError;
use crate::vertex_gemini_transcription::request::{
    complete_url, map_openai_params, transcription_request, validate_environment,
};
use crate::vertex_gemini_transcription::response::{
    InputTokenDetails, TranscriptionUsage, UsageKind, decode_vertex_gemini_transcription,
};
use crate::vertex_recognition::response::TranscriptionTask;

fn audio() -> ProcessedAudioFile {
    ProcessedAudioFile {
        file_content: b"audio".to_vec(),
        filename: "clip.wav".to_owned(),
        content_type: "audio/wav".to_owned(),
    }
}

fn object(source: &str) -> Object {
    serde_json::from_str(source).unwrap()
}

fn reply(status: u16, body: &str) -> UpstreamResponse {
    UpstreamResponse {
        status,
        headers: HeaderMap::new(),
        body: body.as_bytes().to_vec(),
    }
}

fn answer(status: u16, body: &str) -> Object {
    let decoded = decode_vertex_gemini_transcription(&reply(status, body)).unwrap();
    serde_json::from_value(serde_json::to_value(decoded).unwrap()).unwrap()
}

#[rstest]
#[case::default_location(
    None,
    None,
    "vertex_ai/gemini-2.0-flash",
    "https://aiplatform.googleapis.com/v1/projects/my-project/locations/global/publishers/google/models/gemini-2.0-flash:generateContent"
)]
#[case::global(
    None,
    Some("global"),
    "vertex_ai/gemini-2.0-flash",
    "https://aiplatform.googleapis.com/v1/projects/my-project/locations/global/publishers/google/models/gemini-2.0-flash:generateContent"
)]
#[case::region(
    None,
    Some("europe-west1"),
    "gemini-2.0-flash",
    "https://europe-west1-aiplatform.googleapis.com/v1/projects/my-project/locations/europe-west1/publishers/google/models/gemini-2.0-flash:generateContent"
)]
#[case::geography(
    None,
    Some("us"),
    "gemini-2.0-flash",
    "https://aiplatform.us.rep.googleapis.com/v1/projects/my-project/locations/us/publishers/google/models/gemini-2.0-flash:generateContent"
)]
#[case::custom_base(
    Some("https://proxy.example///"),
    None,
    "gemini-2.0-flash",
    "https://proxy.example/v1/projects/my-project/locations/global/publishers/google/models/gemini-2.0-flash:generateContent"
)]
#[case::custom_base_keeps_region(
    Some("https://proxy.example"),
    Some("us-central1"),
    "gemini-2.0-flash",
    "https://proxy.example/v1/projects/my-project/locations/us-central1/publishers/google/models/gemini-2.0-flash:generateContent"
)]
fn transcription_url_cases(
    #[case] api_base: Option<&str>,
    #[case] location: Option<&str>,
    #[case] model: &str,
    #[case] expected: &str,
) {
    assert_eq!(
        complete_url(api_base, location, Some("my-project"), None, model).unwrap(),
        expected
    );
}

#[test]
fn a_credential_project_fills_a_missing_project() {
    let url = complete_url(
        None,
        None,
        None,
        Some("credential-project"),
        "gemini-2.0-flash",
    )
    .unwrap();
    assert_eq!(
        url,
        "https://aiplatform.googleapis.com/v1/projects/credential-project/locations/global/publishers/google/models/gemini-2.0-flash:generateContent"
    );
}

#[test]
fn a_request_project_wins_over_the_credential_project() {
    let url = complete_url(
        None,
        None,
        Some("request-project"),
        Some("credential-project"),
        "gemini-2.0-flash",
    )
    .unwrap();
    assert!(url.contains("/projects/request-project/"));
}

#[test]
fn a_missing_project_is_required() {
    let error = complete_url(None, None, None, None, "gemini-2.0-flash").unwrap_err();
    assert!(matches!(
        error,
        VertexGeminiTranscriptionError::ProjectRequired
    ));
    assert_eq!(error.status_code(), Some(400));
}

#[rstest]
#[case::empty_project(
    None,
    None,
    Some(""),
    "the Vertex AI Gemini transcription request is not valid"
)]
#[case::parent_project(
    None,
    None,
    Some(".."),
    "the Vertex AI Gemini transcription request is not valid"
)]
#[case::slash_project(
    None,
    None,
    Some("bad/project"),
    "the Vertex AI Gemini transcription request is not valid"
)]
#[case::question_project(
    None,
    None,
    Some("bad?project"),
    "the Vertex AI Gemini transcription request is not valid"
)]
#[case::hash_project(
    None,
    None,
    Some("bad#project"),
    "the Vertex AI Gemini transcription request is not valid"
)]
#[case::backslash_project(
    None,
    None,
    Some("bad\\project"),
    "the Vertex AI Gemini transcription request is not valid"
)]
#[case::colon_project(
    None,
    None,
    Some("bad:project"),
    "the Vertex AI Gemini transcription request is not valid"
)]
#[case::space_project(
    None,
    None,
    Some("bad project"),
    "the Vertex AI Gemini transcription request is not valid"
)]
#[case::tab_project(
    None,
    None,
    Some("bad\tproject"),
    "the Vertex AI Gemini transcription request is not valid"
)]
#[case::newline_project(
    None,
    None,
    Some("bad\nproject"),
    "the Vertex AI Gemini transcription request is not valid"
)]
#[case::carriage_return_project(
    None,
    None,
    Some("bad\rproject"),
    "the Vertex AI Gemini transcription request is not valid"
)]
#[case::uppercase_location(
    None,
    Some("US"),
    Some("my-project"),
    "the Vertex location is not valid: "
)]
#[case::slash_location(
    None,
    Some("us/"),
    Some("my-project"),
    "the Vertex location is not valid: "
)]
#[case::digits_first_location(
    None,
    Some("1us"),
    Some("my-project"),
    "the Vertex location is not valid: "
)]
fn unsafe_requests_are_rejected(
    #[case] api_base: Option<&str>,
    #[case] location: Option<&str>,
    #[case] project: Option<&str>,
    #[case] message: &str,
) {
    let error = complete_url(api_base, location, project, None, "gemini-2.0-flash").unwrap_err();
    assert!(error.to_string().starts_with(message), "{error}");
    assert_eq!(error.status_code(), Some(400));
}

#[test]
fn the_model_loses_one_vertex_ai_prefix() {
    let url = complete_url(
        None,
        None,
        Some("my-project"),
        None,
        "vertex_ai/vertex_ai/model",
    )
    .unwrap();
    assert!(url.ends_with("/publishers/google/models/vertex_ai/model:generateContent"));
}

#[rstest]
#[case::supported_language(r#"{"language": "en"}"#, "{}", r#"{"language": "en"}"#)]
#[case::supported_text(
    r#"{"response_format": "text"}"#,
    "{}",
    r#"{"response_format": "text"}"#
)]
#[case::supported_json(
    r#"{"response_format": "json"}"#,
    "{}",
    r#"{"response_format": "json"}"#
)]
#[case::null_format(r#"{"response_format": null}"#, "{}", r#"{"response_format": null}"#)]
#[case::unsupported_name(r#"{"temperature": 1}"#, "{}", "{}")]
#[case::overwrites_optional(
    r#"{"language": "en"}"#,
    r#"{"language": "de"}"#,
    r#"{"language": "en"}"#
)]
fn map_openai_params_cases(
    #[case] non_default: &str,
    #[case] optional: &str,
    #[case] expected: &str,
) {
    let mapped = map_openai_params(
        &object(non_default),
        &object(optional),
        DropParams::default(),
    );
    assert_eq!(mapped.unwrap(), object(expected));
}

#[test]
fn an_unsupported_response_format_is_rejected_without_drop_params() {
    let error = map_openai_params(
        &object(r#"{"response_format": "srt"}"#),
        &Object::new(),
        DropParams::default(),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        VertexGeminiTranscriptionError::Shared { .. }
    ));
    assert_eq!(error.status_code(), Some(400));
}

#[test]
fn an_unsupported_response_format_leaves_a_supplied_option_too() {
    let error = map_openai_params(
        &Object::new(),
        &object(r#"{"response_format": "vtt"}"#),
        DropParams::default(),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        VertexGeminiTranscriptionError::Shared { .. }
    ));
}

#[rstest]
#[case::request_drop(DropParams {
    global: false,
    request: true,
})]
#[case::global_drop(DropParams {
    global: true,
    request: false,
})]
fn drop_params_removes_the_unsupported_response_format(#[case] policy: DropParams) {
    let mapped = map_openai_params(
        &object(r#"{"response_format": "srt", "language": "en"}"#),
        &Object::new(),
        policy,
    )
    .unwrap();
    assert_eq!(mapped, object(r#"{"language": "en"}"#));
}

#[test]
fn header_overrides_replace_caller_values() {
    let mut caller = HeaderMap::new();
    caller.insert(AUTHORIZATION, HeaderValue::from_static("Bearer caller-key"));
    caller.insert(
        HeaderName::from_static("x-goog-user-project"),
        HeaderValue::from_static("caller-project"),
    );
    caller.insert(CONTENT_TYPE, HeaderValue::from_static("text/plain"));
    caller.insert(
        HeaderName::from_static("x-extra"),
        HeaderValue::from_static("kept"),
    );
    let headers =
        validate_environment(caller, &SecretString::from("token"), "forced-project").unwrap();
    assert_eq!(
        headers.get(AUTHORIZATION).unwrap().to_str().unwrap(),
        "Bearer token"
    );
    assert_eq!(
        headers
            .get("x-goog-user-project")
            .unwrap()
            .to_str()
            .unwrap(),
        "forced-project"
    );
    assert_eq!(
        headers.get(CONTENT_TYPE).unwrap().to_str().unwrap(),
        "application/json"
    );
    assert_eq!(headers.get("x-extra").unwrap().to_str().unwrap(), "kept");
    assert_eq!(headers.len(), 4);
}

#[rstest]
#[case::absent(None, &[])]
#[case::empty(Some(Json::Text(String::new())), &[])]
#[case::not_text(Some(Json::Bool(true)), &[])]
#[case::bare(Some(Json::Text("en".to_owned())), &["en-US"])]
#[case::qualified(Some(Json::Text("en-gb".to_owned())), &["en-gb"])]
#[case::unknown(Some(Json::Text("zz".to_owned())), &["zz"])]
fn language_configs(#[case] language: Option<Json>, #[case] expected: &[&str]) {
    let request = transcription_request(&audio(), language.as_ref());
    let codes = &request
        .generation_config
        .audio_transcription_config
        .language_codes;
    assert_eq!(codes.as_slice(), expected);
}

#[test]
fn the_request_holds_a_single_inline_audio_part() {
    let request = transcription_request(&audio(), None);
    assert_eq!(
        serde_json::to_string(&request).unwrap(),
        r#"{"contents":[{"role":"user","parts":[{"inlineData":{"mimeType":"audio/wav","data":"YXVkaW8="}}]}],"generationConfig":{"audioTranscriptionConfig":{}}}"#
    );
}

#[test]
fn the_request_carries_no_transcription_prompt() {
    let encoded = serde_json::to_string(&transcription_request(
        &audio(),
        Some(&Json::Text("en".to_owned())),
    ))
    .unwrap();
    assert!(!encoded.contains("\"text\""));
    assert_eq!(
        encoded,
        r#"{"contents":[{"role":"user","parts":[{"inlineData":{"mimeType":"audio/wav","data":"YXVkaW8="}}]}],"generationConfig":{"audioTranscriptionConfig":{"languageCodes":["en-US"]}}}"#
    );
}

#[test]
fn the_request_uses_the_processed_mime_type() {
    let mut processed = audio();
    processed.content_type = "audio/mpeg".to_owned();
    let encoded = serde_json::to_string(&transcription_request(&processed, None)).unwrap();
    assert!(encoded.contains(r#""mimeType":"audio/mpeg""#));
}

#[test]
fn all_candidate_text_is_joined_with_spaces() {
    let body = r#"{"candidates":[
        {"content":{"parts":[{"text":"hello"},{"text":""},{"text":null}]},"finishReason":"STOP"},
        {"content":{"parts":[{"text":"there"},{"text":"world"}]}},
        {"content":null},
        {"finishReason":"MAX_TOKENS"}
    ]}"#;
    assert_eq!(answer(200, body)["text"], "hello there world");
}

#[test]
fn whitespace_only_text_is_kept() {
    let body = r#"{"candidates":[{"content":{"parts":[{"text":"   "},{"text":"a b"}]}}]}"#;
    assert_eq!(answer(200, body)["text"], "    a b");
}

#[test]
fn missing_candidates_and_content_default_to_empty() {
    assert_eq!(answer(200, "{}")["text"], "");
    assert_eq!(answer(200, r#"{"candidates":[]}"#)["text"], "");
    assert_eq!(
        answer(200, r#"{"candidates":[{"content":{}},{}]}"#)["text"],
        ""
    );
    assert_eq!(
        answer(200, r#"{"candidates":[{"content":{"parts":[]}}]}"#)["text"],
        ""
    );
}

#[test]
fn unknown_response_fields_are_ignored() {
    let body = r#"{"candidates":[{"content":{"parts":[{"text":"kept","thought":true}],"role":"model"},"safetyRatings":[]}],"usageMetadata":{"promptTokenCount":3,"trafficType":"ON_DEMAND"},"responseId":"r1"}"#;
    let value = answer(200, body);
    assert_eq!(value["text"], "kept");
    assert_eq!(value["usage"]["input_tokens"], 3);
    assert_eq!(value["usage"]["input_token_details"]["audio_tokens"], 0);
    let encoded =
        serde_json::to_string(&decode_vertex_gemini_transcription(&reply(200, body)).unwrap())
            .unwrap();
    assert!(!encoded.contains("responseId"));
    assert!(!encoded.contains("safetyRatings"));
}

#[test]
fn usage_is_absent_without_usage_metadata() {
    assert!(answer(200, "{}").get("usage").is_none());
    assert!(
        answer(200, r#"{"candidates":[],"usageMetadata":null}"#)
            .get("usage")
            .is_none()
    );
}

#[test]
fn an_empty_usage_object_yields_zero_counts() {
    let value = answer(200, r#"{"candidates":[],"usageMetadata":{}}"#);
    assert_eq!(value["usage"]["type"], "tokens");
    assert_eq!(value["usage"]["input_tokens"], 0);
    assert_eq!(value["usage"]["output_tokens"], 0);
    assert_eq!(value["usage"]["total_tokens"], 0);
    assert_eq!(value["usage"]["input_token_details"]["audio_tokens"], 0);
    assert_eq!(value["usage"]["input_token_details"]["text_tokens"], 0);
}

#[test]
fn audio_tokens_count_only_the_exact_audio_modality() {
    let body = r#"{"candidates":[],"usageMetadata":{"promptTokenCount":10,"promptTokensDetails":[{"modality":"AUDIO","tokenCount":6},{"modality":"audio","tokenCount":3},{"modality":"TEXT","tokenCount":2},{"tokenCount":5}]}}"#;
    let value = answer(200, body);
    assert_eq!(value["usage"]["input_token_details"]["audio_tokens"], 6);
    assert_eq!(value["usage"]["input_token_details"]["text_tokens"], 4);
}

#[test]
fn text_tokens_subtract_audio_without_clamping() {
    let body = r#"{"candidates":[],"usageMetadata":{"promptTokenCount":2,"candidatesTokenCount":5,"totalTokenCount":9,"promptTokensDetails":[{"modality":"AUDIO","tokenCount":7}]}}"#;
    let value = answer(200, body);
    assert_eq!(value["usage"]["input_token_details"]["audio_tokens"], 7);
    assert_eq!(value["usage"]["input_token_details"]["text_tokens"], -5);
}

#[test]
fn provider_total_and_candidate_counts_are_preserved() {
    let body = r#"{"candidates":[],"usageMetadata":{"promptTokenCount":2,"candidatesTokenCount":5,"totalTokenCount":9}}"#;
    let value = answer(200, body);
    assert_eq!(value["usage"]["input_tokens"], 2);
    assert_eq!(value["usage"]["output_tokens"], 5);
    assert_eq!(value["usage"]["total_tokens"], 9);
}

#[test]
fn a_null_candidates_field_is_a_shape_error() {
    let error =
        decode_vertex_gemini_transcription(&reply(200, r#"{"candidates":null}"#)).unwrap_err();
    assert!(matches!(
        error,
        VertexGeminiTranscriptionError::Shape { .. }
    ));
    assert_eq!(error.status_code(), None);
}

#[test]
fn a_null_parts_field_is_a_shape_error() {
    let error = decode_vertex_gemini_transcription(&reply(
        200,
        r#"{"candidates":[{"content":{"parts":null}}]}"#,
    ))
    .unwrap_err();
    assert!(matches!(
        error,
        VertexGeminiTranscriptionError::Shape { .. }
    ));
}

#[rstest]
#[case::fractional_count(r#"{"candidates":[],"usageMetadata":{"promptTokenCount":7.5}}"#)]
#[case::text_count(r#"{"candidates":[],"usageMetadata":{"promptTokenCount":"7"}}"#)]
#[case::null_count(r#"{"candidates":[],"usageMetadata":{"candidatesTokenCount":null}}"#)]
#[case::list_details(r#"{"candidates":[],"usageMetadata":{"promptTokensDetails":{}}}"#)]
fn wrong_count_types_are_shape_errors(#[case] body: &str) {
    let error = decode_vertex_gemini_transcription(&reply(200, body)).unwrap_err();
    assert!(matches!(
        error,
        VertexGeminiTranscriptionError::Shape { .. }
    ));
}

#[rstest]
#[case::sum(
    r#"{"candidates":[],"usageMetadata":{"promptTokensDetails":[{"modality":"AUDIO","tokenCount":9223372036854775807},{"modality":"AUDIO","tokenCount":1}]}}"#
)]
#[case::subtraction(
    r#"{"candidates":[],"usageMetadata":{"promptTokenCount":-9223372036854775808,"promptTokensDetails":[{"modality":"AUDIO","tokenCount":1}]}}"#
)]
fn a_usage_count_overflow_is_refused(#[case] body: &str) {
    let error = decode_vertex_gemini_transcription(&reply(200, body)).unwrap_err();
    assert!(matches!(
        error,
        VertexGeminiTranscriptionError::CountOverflow
    ));
    assert_eq!(error.status_code(), None);
}

#[test]
fn a_non_json_reply_keeps_the_reply_status() {
    let error = decode_vertex_gemini_transcription(&reply(201, "not json")).unwrap_err();
    assert!(matches!(
        error,
        VertexGeminiTranscriptionError::NotJson { status: 201, .. }
    ));
    assert_eq!(error.status_code(), Some(201));
    let trailing =
        decode_vertex_gemini_transcription(&reply(200, r#"{"candidates":[]} tail"#)).unwrap_err();
    assert!(matches!(
        trailing,
        VertexGeminiTranscriptionError::NotJson { .. }
    ));
}

// a proxy's plain-text status line starts with a JSON number, but LiteLLM's json() still refuses the body
#[test]
fn a_body_that_is_not_one_json_document_keeps_the_reply_status() {
    let error = decode_vertex_gemini_transcription(&reply(200, "200 OK")).unwrap_err();
    assert!(matches!(
        error,
        VertexGeminiTranscriptionError::NotJson { status: 200, .. }
    ));
    assert_eq!(error.status_code(), Some(200));
    let trailing =
        decode_vertex_gemini_transcription(&reply(201, r#"{"candidates":null} tail"#)).unwrap_err();
    assert!(matches!(
        trailing,
        VertexGeminiTranscriptionError::NotJson { status: 201, .. }
    ));
}

#[test]
fn a_reply_over_the_size_limit_is_refused() {
    let at_limit = format!("{{}}{}", " ".repeat(MAX_RESPONSE_BYTES - 2));
    assert_eq!(at_limit.len(), MAX_RESPONSE_BYTES);
    decode_vertex_gemini_transcription(&reply(200, &at_limit)).unwrap();
    let error =
        decode_vertex_gemini_transcription(&reply(200, &format!("{at_limit} "))).unwrap_err();
    assert!(matches!(
        error,
        VertexGeminiTranscriptionError::TooLarge { size } if size == MAX_RESPONSE_BYTES + 1
    ));
    assert_eq!(error.status_code(), None);
}

#[test]
fn the_response_hides_the_provider_envelope() {
    let body = r#"{"candidates":[{"content":{"parts":[{"text":"hi"}]}}],"usageMetadata":{"promptTokenCount":10,"candidatesTokenCount":4,"totalTokenCount":14,"promptTokensDetails":[{"modality":"AUDIO","tokenCount":6}]}}"#;
    let decoded = decode_vertex_gemini_transcription(&reply(200, body)).unwrap();
    assert_eq!(decoded.task, TranscriptionTask::Transcribe);
    assert_eq!(
        decoded.usage,
        Some(TranscriptionUsage {
            kind: UsageKind::Tokens,
            input_tokens: 10,
            output_tokens: 4,
            total_tokens: 14,
            input_token_details: InputTokenDetails {
                audio_tokens: 6,
                text_tokens: 4,
            },
        })
    );
    assert_eq!(
        serde_json::to_string(&decoded).unwrap(),
        r#"{"text":"hi","task":"transcribe","usage":{"type":"tokens","input_tokens":10,"output_tokens":4,"total_tokens":14,"input_token_details":{"audio_tokens":6,"text_tokens":4}}}"#
    );
}

const VALID_SEEDS: [(&str, &str, Option<[i64; 5]>); 5] = [
    ("all-candidates.json", "hello there world", None),
    ("negative-text-tokens.json", "", Some([2, 5, 9, 7, -5])),
    ("sparse-candidates.json", "ok", None),
    ("unknown-fields.json", "kept", Some([3, 0, 0, 0, 3])),
    ("usage-tokens.json", "hi", Some([10, 4, 14, 6, 4])),
];

const INVALID_SEEDS: [&str; 2] = ["wrong-count-type.json", "wrong-part-type.json"];

#[rstest]
fn replays_the_vertex_gemini_transcription_corpus(
    #[files("fuzz/corpus/vertex_gemini_transcription/*")] path: PathBuf,
) {
    let name = path.file_name().unwrap().to_str().unwrap();
    let body = fs::read_to_string(&path).unwrap();
    if INVALID_SEEDS.contains(&name) {
        let error = decode_vertex_gemini_transcription(&reply(200, &body)).unwrap_err();
        assert!(matches!(
            error,
            VertexGeminiTranscriptionError::Shape { .. }
        ));
        return;
    }
    let (_, text, usage) = VALID_SEEDS
        .iter()
        .find(|seed| seed.0 == name)
        .unwrap_or_else(|| panic!("the {name} seed has no expectation"));
    let value = answer(200, &body);
    assert_eq!(value["text"], *text);
    match usage {
        Some([input, output, total, audio, text_tokens]) => {
            assert_eq!(value["usage"]["input_tokens"], *input);
            assert_eq!(value["usage"]["output_tokens"], *output);
            assert_eq!(value["usage"]["total_tokens"], *total);
            assert_eq!(
                value["usage"]["input_token_details"]["audio_tokens"],
                *audio
            );
            assert_eq!(
                value["usage"]["input_token_details"]["text_tokens"],
                *text_tokens
            );
        }
        None => assert!(value.get("usage").is_none()),
    }
}

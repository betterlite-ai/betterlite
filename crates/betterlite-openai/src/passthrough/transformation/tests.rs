#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;
use serde_json::{Value, json};
use thiserror::Error;
use url::Url;

use crate::error::ProviderError;
use crate::openai_base_config::{Headers, Object};
use crate::passthrough::transformation::types::{
    MAX_COLLECTED_BYTES, PassthroughError, PassthroughLogging, PassthroughSigning,
    PassthroughTarget, PassthroughUrlRequest, RelayCallType, RelayShape, RelayedResponse,
};
use crate::passthrough::transformation::{
    BasePassthroughConfig, convert_raw_bytes_to_lines, logged_relay_shape, model_group_from,
    relayed_body, relayed_json_object, replace_path_segment, strip_leading_model_segment,
};

fn test_object<I>(pairs: I) -> Object
where
    I: IntoIterator<Item = (&'static str, Value)>,
{
    pairs
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

fn metadata_object(metadata: Value) -> Object {
    test_object([("litellm_metadata", metadata)])
}

fn model_group_metadata(model_group: Value) -> Object {
    metadata_object(Value::Object(test_object([("model_group", model_group)])))
}

fn metadata_with_extra_field() -> Object {
    let metadata = test_object([("model_group", json!("group-a")), ("other", json!(1_i64))]);
    metadata_object(Value::Object(metadata))
}

fn passthrough_target() -> PassthroughTarget<'static> {
    PassthroughTarget {
        model: "model",
        custom_llm_provider: "provider",
        endpoint: "endpoint",
    }
}

#[derive(Debug, Error)]
#[error("{message}")]
struct TestError {
    message: String,
}

impl From<ProviderError> for TestError {
    fn from(error: ProviderError) -> Self {
        Self {
            message: error.message,
        }
    }
}

impl From<PassthroughError> for TestError {
    fn from(error: PassthroughError) -> Self {
        Self {
            message: error.to_string(),
        }
    }
}

struct TestConfig;

impl BasePassthroughConfig for TestConfig {
    type Error = TestError;
    type Logged = String;

    fn is_streaming_request(&self, _endpoint: &str, request_data: &Object) -> bool {
        request_data.get("stream") == Some(&Value::Bool(true))
    }

    fn get_complete_url(
        &self,
        request: &PassthroughUrlRequest<'_>,
    ) -> Result<(Url, String), Self::Error> {
        let base = request.api_base.unwrap_or("https://api.test");
        let url = self.format_url(request.target.endpoint, base, request.request_query_params)?;
        Ok((url, base.to_owned()))
    }

    fn handle_collected_chunks(
        &self,
        all_chunks: &[String],
        _logging_obj: &mut dyn PassthroughLogging,
        target: &PassthroughTarget<'_>,
    ) -> Option<Self::Logged> {
        Some(format!(
            "{}|{}|{}|{}",
            target.model,
            target.custom_llm_provider,
            target.endpoint,
            all_chunks.join(";")
        ))
    }
}

struct DefaultConfig;

impl BasePassthroughConfig for DefaultConfig {
    type Error = TestError;
    type Logged = Object;

    fn is_streaming_request(&self, _endpoint: &str, _request_data: &Object) -> bool {
        false
    }

    fn get_complete_url(
        &self,
        _request: &PassthroughUrlRequest<'_>,
    ) -> Result<(Url, String), Self::Error> {
        let url = self.format_url("", "https://api.test", None)?;
        Ok((url, String::new()))
    }
}

struct RecordingLogging {
    call_type: Option<RelayCallType>,
}

impl PassthroughLogging for RecordingLogging {
    fn set_call_type(&mut self, call_type: RelayCallType) {
        self.call_type = Some(call_type);
    }
}

fn shape(path_suffix: &'static str, call_type: RelayCallType) -> RelayShape<'static, Object> {
    RelayShape {
        path_suffix,
        call_type,
        parse: |body| Some(body.clone()),
    }
}

fn failing_shape() -> RelayShape<'static, Object> {
    RelayShape {
        path_suffix: "/responses",
        call_type: RelayCallType::Aresponses,
        parse: |_body| None,
    }
}

fn responses_first() -> [RelayShape<'static, Object>; 2] {
    [
        shape("/responses", RelayCallType::Aresponses),
        shape("/v1/responses", RelayCallType::Aembedding),
    ]
}

fn v1_first() -> [RelayShape<'static, Object>; 2] {
    [
        shape("/v1/responses", RelayCallType::Aembedding),
        shape("/responses", RelayCallType::Aresponses),
    ]
}

#[rstest]
#[case::valid(model_group_metadata(json!("group-a")), "group-a")]
#[case::missing_metadata(test_object([]), "")]
#[case::empty_metadata(metadata_object(Value::Object(Object::new())), "")]
#[case::not_an_object(metadata_object(json!("group-a")), "")]
#[case::array_metadata(metadata_object(json!(["group-a"])), "")]
#[case::wrong_field_type(model_group_metadata(json!(5_i64)), "")]
#[case::extra_fields(metadata_with_extra_field(), "group-a")]
fn model_group_from_reads_litellm_metadata(#[case] params: Object, #[case] expected: &str) {
    assert_eq!(model_group_from(&params), expected);
}

#[rstest]
#[case::leading_slash("/model/chat/completions", &["model"], "chat/completions")]
#[case::repeated_slashes("//model/chat", &["model"], "chat")]
#[case::exact_model("/model", &["model"], "")]
#[case::other_prefix("/other/chat", &["model"], "other/chat")]
#[case::partial_segment("/model2/chat", &["model"], "model2/chat")]
#[case::empty_name_skipped("/model/chat", &["", "model"], "chat")]
#[case::first_match_wins("/group/model/chat", &["group", "model"], "model/chat")]
#[case::no_names("/model/chat", &[], "model/chat")]
fn strip_leading_model_segment_removes_a_known_first_segment(
    #[case] endpoint: &str,
    #[case] names: &[&str],
    #[case] expected: &str,
) {
    let model_names: Vec<String> = names.iter().map(|name| (*name).to_owned()).collect();
    assert_eq!(
        strip_leading_model_segment(endpoint, &model_names),
        expected
    );
}

#[rstest]
#[case::slash_bounded("/v1/chat/completions", "chat", "/v1/x/completions")]
#[case::at_path_start("chat/completions", "chat", "x/completions")]
#[case::before_colon("/v1/chat:stream", "chat", "/v1/x:stream")]
#[case::longer_segment("/v1/chatty", "chat", "/v1/chatty")]
#[case::preceded_by_text("/v1/xchat", "chat", "/v1/xchat")]
#[case::followed_by_query("/v1/chat?x=1", "chat", "/v1/chat?x=1")]
#[case::every_occurrence("/a/chat/b/chat", "chat", "/a/x/b/x")]
#[case::literal_dot("/a/c.t/b", "c.t", "/a/x/b")]
#[case::dot_is_literal("/a/cxt/b", "c.t", "/a/cxt/b")]
fn replace_path_segment_matches_whole_segments(
    #[case] endpoint: &str,
    #[case] segment: &str,
    #[case] expected: &str,
) {
    assert_eq!(replace_path_segment(endpoint, segment, "x"), expected);
}

#[rstest]
#[case::object(200, br#"{"id":"r"}"#.to_vec(), Some(test_object([("id", json!("r"))])))]
#[case::utf8_bom(200, b"\xef\xbb\xbf{\"id\":\"r\"}".to_vec(), Some(test_object([("id", json!("r"))])))]
#[case::utf16_bom_is_not_json(200, b"\xff\xfe{\x00}\x00".to_vec(), None)]
#[case::utf16_le_without_bom_is_not_json(
    200,
    b"{\x00\"\x00a\x00\"\x00:\x001\x00}\x00".to_vec(),
    None
)]
#[case::other_status(201, br#"{"id":"r"}"#.to_vec(), None)]
#[case::array(200, b"[1]".to_vec(), None)]
#[case::text(200, b"\"value\"".to_vec(), None)]
#[case::invalid_json(200, b"not json".to_vec(), None)]
fn relayed_json_object_projects_only_a_200_object(
    #[case] status: u16,
    #[case] body: Vec<u8>,
    #[case] expected: Option<Object>,
) {
    assert_eq!(relayed_json_object(status, &body), expected);
}

#[rstest]
#[case::object(br#"{"id":"r"}"#.to_vec(), Value::Object(test_object([("id", json!("r"))])))]
#[case::utf8_bom(
    b"\xef\xbb\xbf{\"id\":\"r\"}".to_vec(),
    Value::Object(test_object([("id", json!("r"))]))
)]
#[case::array(b"[1]".to_vec(), json!("[1]"))]
#[case::json_text(b"\"value\"".to_vec(), json!("\"value\""))]
#[case::invalid_json(b"not json".to_vec(), json!("not json"))]
#[case::utf16_le_without_bom(
    b"{\x00\"\x00a\x00\"\x00:\x001\x00}\x00".to_vec(),
    json!("{\u{0}\"\u{0}a\u{0}\"\u{0}:\u{0}1\u{0}}\u{0}")
)]
fn relayed_body_falls_back_to_the_response_text(#[case] body: Vec<u8>, #[case] expected: Value) {
    let relayed = relayed_body(&body).unwrap();
    assert_eq!(serde_json::to_value(relayed).unwrap(), expected);
}

#[test]
fn relayed_body_refuses_bytes_that_are_not_utf8() {
    assert!(matches!(
        relayed_body(b"\xff\xfe"),
        Err(PassthroughError::InvalidUtf8 { .. })
    ));
}

#[test]
fn logged_relay_shape_picks_the_first_suffix_in_list_order() {
    let mut logging = RecordingLogging { call_type: None };
    let parsed = logged_relay_shape(
        &responses_first(),
        200,
        br#"{"id":"r"}"#,
        &mut logging,
        "v1/responses",
    );
    assert_eq!(parsed, Some(test_object([("id", json!("r"))])));
    assert_eq!(logging.call_type, Some(RelayCallType::Aresponses));
}

#[test]
fn logged_relay_shape_respects_a_reordered_shape_list() {
    let mut logging = RecordingLogging { call_type: None };
    let parsed = logged_relay_shape(
        &v1_first(),
        200,
        br#"{"id":"r"}"#,
        &mut logging,
        "v1/responses/",
    );
    assert!(parsed.is_some());
    assert_eq!(logging.call_type, Some(RelayCallType::Aembedding));
}

#[test]
fn logged_relay_shape_ignores_a_non_200_response() {
    let mut logging = RecordingLogging { call_type: None };
    assert!(
        logged_relay_shape(
            &responses_first(),
            500,
            br#"{"id":"r"}"#,
            &mut logging,
            "responses"
        )
        .is_none()
    );
    assert_eq!(logging.call_type, None);
}

#[test]
fn logged_relay_shape_returns_none_without_a_matching_suffix() {
    let mut logging = RecordingLogging { call_type: None };
    assert!(
        logged_relay_shape(
            &responses_first(),
            200,
            br#"{"id":"r"}"#,
            &mut logging,
            "v1/embeddings"
        )
        .is_none()
    );
    assert_eq!(logging.call_type, None);
}

#[test]
fn logged_relay_shape_returns_none_for_a_non_object_body() {
    let mut logging = RecordingLogging { call_type: None };
    assert!(
        logged_relay_shape(&responses_first(), 200, b"[1]", &mut logging, "responses").is_none()
    );
    assert_eq!(logging.call_type, None);
}

#[test]
fn logged_relay_shape_keeps_the_call_type_when_parsing_fails() {
    let mut logging = RecordingLogging { call_type: None };
    assert!(
        logged_relay_shape(
            &[failing_shape()],
            200,
            br#"{"id":"r"}"#,
            &mut logging,
            "responses"
        )
        .is_none()
    );
    assert_eq!(logging.call_type, None);
}

#[rstest]
#[case::split_code_point(vec![&b"h\xc3"[..], &b"\xa9llo\n"[..]], vec!["h\u{e9}llo"])]
#[case::empty_lines(vec![&b"a\n\n b \n"[..]], vec!["a", "b"])]
#[case::carriage_returns(vec![&b"a\r\nb\r\n"[..]], vec!["a", "b"])]
#[case::record_separators(vec![&b"\x1edata: x\x1f\n"[..]], vec!["data: x"])]
#[case::separator_only_line(vec![&b"\x1e\nvalue"[..]], vec!["value"])]
#[case::no_chunks(vec![], vec![])]
fn convert_raw_bytes_to_lines_joins_before_decoding(
    #[case] chunks: Vec<&[u8]>,
    #[case] expected: Vec<&str>,
) {
    let lines = convert_raw_bytes_to_lines(&chunks).unwrap();
    assert_eq!(lines, expected);
}

#[test]
fn convert_raw_bytes_to_lines_refuses_bytes_that_are_not_utf8() {
    assert!(matches!(
        convert_raw_bytes_to_lines(&[&[0xff, 0xfe]]),
        Err(PassthroughError::InvalidUtf8 { .. })
    ));
}

#[test]
fn raw_bytes_collector_joins_chunks_before_decoding() {
    let config = TestConfig;
    let target = passthrough_target();
    let mut collector = config.create_stream_collector(&target);
    collector.add(b"h\xc3");
    collector.add(b"\xa9llo\n");
    let mut logging = RecordingLogging { call_type: None };
    let logged = collector.build_logged_response(&mut logging).unwrap();
    assert_eq!(
        logged,
        Some("model|provider|endpoint|h\u{e9}llo".to_owned())
    );
}

#[test]
fn raw_bytes_collector_stops_at_the_collection_cap() {
    let config = TestConfig;
    let target = passthrough_target();
    let mut collector = config.create_stream_collector(&target);
    let chunk = vec![b'a'; MAX_COLLECTED_BYTES];
    collector.add(&chunk);
    collector.add(b"b");
    let mut logging = RecordingLogging { call_type: None };
    assert_eq!(collector.build_logged_response(&mut logging).unwrap(), None);
}

#[test]
fn raw_bytes_collector_reports_bytes_that_are_not_utf8() {
    let config = TestConfig;
    let target = passthrough_target();
    let mut collector = config.create_stream_collector(&target);
    collector.add(b"\xff\xfe");
    let mut logging = RecordingLogging { call_type: None };
    assert!(matches!(
        collector.build_logged_response(&mut logging),
        Err(PassthroughError::InvalidUtf8 { .. })
    ));
}

#[test]
fn default_handlers_log_nothing() {
    let config = DefaultConfig;
    let target = passthrough_target();
    let response = RelayedResponse {
        status_code: 200,
        body: br#"{"id":"r"}"#,
    };
    let mut logging = RecordingLogging { call_type: None };
    let request_data = Object::new();
    assert!(
        config
            .logging_non_streaming_response(&target, &response, &request_data, &mut logging)
            .is_none()
    );
    assert!(
        config
            .handle_collected_chunks(&[], &mut logging, &target)
            .is_none()
    );
}

#[test]
fn default_sign_request_keeps_the_headers() {
    let config = DefaultConfig;
    let headers = Headers::from([("Authorization".to_owned(), "Bearer key".to_owned())]);
    let litellm_params = Object::new();
    let signing = PassthroughSigning {
        litellm_params: &litellm_params,
        request_data: None,
        api_base: "https://api.test",
        model: None,
    };
    let (signed, body) = config.sign_request(headers.clone(), &signing).unwrap();
    assert_eq!(signed, headers);
    assert!(body.is_none());
}

#[rstest]
#[case::trailing_slashes("https://api.test///", "/v1/chat", "https://api.test/v1/chat")]
#[case::leading_slashes("https://api.test", "///v1/chat", "https://api.test/v1/chat")]
#[case::plain("https://api.test", "v1/chat", "https://api.test/v1/chat")]
fn format_url_trims_the_join_slashes(
    #[case] base: &str,
    #[case] endpoint: &str,
    #[case] expected: &str,
) {
    let url = DefaultConfig.format_url(endpoint, base, None).unwrap();
    assert_eq!(url.as_str(), expected);
}

#[rstest]
#[case::none(None)]
#[case::empty(Some(test_object([])))]
fn format_url_keeps_the_base_query_without_parameters(#[case] params: Option<Object>) {
    let url = DefaultConfig
        .format_url("v1/chat", "https://api.test/base?old=1", params.as_ref())
        .unwrap();
    assert_eq!(url.as_str(), "https://api.test/base?old=1/v1/chat");
}

#[test]
fn format_url_replaces_the_query_with_encoded_parameters() {
    let params = test_object([
        ("api-version", json!("2024-02-01")),
        ("limit", json!(10_i64)),
    ]);
    let url = DefaultConfig
        .format_url("v1/chat", "https://api.test/base?old=1", Some(&params))
        .unwrap();
    assert_eq!(
        url.as_str(),
        "https://api.test/base?api-version=2024-02-01&limit=10"
    );
}

#[test]
fn format_url_encodes_query_text_like_urlencode() {
    let params = test_object([("q", json!("a b&c")), ("\u{e9}", json!("\u{fc}"))]);
    let url = DefaultConfig
        .format_url("v1/chat", "https://api.test", Some(&params))
        .unwrap();
    assert_eq!(
        url.as_str(),
        "https://api.test/v1/chat?q=a+b%26c&%C3%A9=%C3%BC"
    );
}

#[test]
fn format_url_rejects_a_query_value_that_is_not_scalar() {
    let params = test_object([("filter", json!(["a"]))]);
    let error = DefaultConfig
        .format_url("v1/chat", "https://api.test", Some(&params))
        .unwrap_err();
    assert_eq!(
        error,
        PassthroughError::QueryValue {
            name: "filter".to_owned()
        }
    );
}

#[test]
fn format_url_reports_a_url_that_does_not_parse() {
    let error = DefaultConfig.format_url("", "", None).unwrap_err();
    assert!(matches!(error, PassthroughError::InvalidUrl { .. }));
    assert!(error.to_string().contains("not valid"));
}

#[rstest]
#[case::headers(
    Headers::from([("x-request-id".to_owned(), "r1".to_owned())]),
    Some(vec![("x-request-id".to_owned(), "r1".to_owned())])
)]
#[case::empty(Headers::new(), Some(Vec::new()))]
fn default_error_class_carries_the_status_message_and_headers(
    #[case] headers: Headers,
    #[case] expected: Option<Vec<(String, String)>>,
) {
    let error = DefaultConfig.get_error_class("upstream failed", 503, &headers);
    assert_eq!(error.status_code, 503);
    assert_eq!(error.message, "upstream failed");
    assert_eq!(error.headers, expected);
    assert!(!error.status_code_is_synthesized);
}

#[rstest]
fn replays_the_passthrough_bytes_corpus(#[files("fuzz/corpus/passthrough_bytes/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let whole = convert_raw_bytes_to_lines(&[&bytes]);
    let chunks: Vec<&[u8]> = bytes.chunks(1).collect();
    match whole {
        Ok(lines) => assert_eq!(convert_raw_bytes_to_lines(&chunks).unwrap(), lines),
        Err(_) => {
            convert_raw_bytes_to_lines(&chunks).unwrap_err();
        }
    }
}

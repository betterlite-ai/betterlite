#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use betterlite_config::sections::studio_identity::Settings;
use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;

use super::credentials::{StudioParams, resolve_api_base, validate_environment};
use super::decode::{
    RESPONSE_BODY_LIMIT, RawResponse, decode_studio_interaction, transform_delete_response,
    transform_get_response, transform_response, transform_streaming_response,
};
use super::error::StudioInteractionError;
use super::records::{CancelInteractionResult, InteractionRecord};
use super::request::{InteractionOptions, transform_request};
use super::urls::{Lifecycle, create_url, lifecycle_request};
use super::{API_REVISION, API_VERSION, SUPPORTED_PARAMS};

const DEFAULT_BASE: &str = "https://generativelanguage.googleapis.com";

fn secret(text: &str) -> SecretString {
    SecretString::from(text)
}

fn settings(key: Option<&str>) -> Settings {
    Settings {
        environment_key: key.map(secret),
        generic_api_key: None,
        api_base: DEFAULT_BASE.to_owned(),
    }
}

fn params(key: Option<&str>) -> StudioParams {
    StudioParams {
        api_key: key.map(secret),
    }
}

fn parse(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn dumped<T>(value: &T) -> Json
where
    T: Serialize,
{
    parse(&serde_json::to_string(value).unwrap())
}

fn options(text: &str) -> InteractionOptions {
    serde_json::from_str(text).unwrap()
}

fn built(model: Option<&str>, agent: Option<&str>, given: &InteractionOptions) -> Json {
    dumped(&transform_request(model, agent, None, given).unwrap())
}

fn headers(pairs: &[(&str, &str)]) -> IndexMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn reply<'raw>(
    status: u16,
    headers: &'raw IndexMap<String, String>,
    body: &'raw str,
) -> RawResponse<'raw> {
    RawResponse {
        status,
        headers,
        body: body.as_bytes(),
    }
}

fn marked(_headers: &IndexMap<String, String>) -> AdditionalHeaders {
    AdditionalHeaders::from([("projected".to_owned(), Json::Bool(true))])
}

#[test]
fn supported_params_keep_the_native_order() {
    assert_eq!(SUPPORTED_PARAMS.len(), 14);
    assert_eq!(SUPPORTED_PARAMS.first(), Some(&"model"));
    assert_eq!(SUPPORTED_PARAMS.last(), Some(&"previous_interaction_id"));
    assert_eq!(SUPPORTED_PARAMS.get(5), Some(&"generation_config"));
    assert_eq!(API_VERSION, "v1beta");
}

#[rstest]
#[case::model_wins(
    Some("gemini/gemini-2.5-flash"),
    Some("deep-research"),
    r#"{"model": "gemini-2.5-flash"}"#
)]
#[case::agent_when_model_empty(Some(""), Some("deep-research"), r#"{"agent": "deep-research"}"#)]
#[case::agent_when_model_absent(None, Some("deep-research"), r#"{"agent": "deep-research"}"#)]
#[case::prefix_only_model_keeps_prefix(Some("gemini/"), None, r#"{"model": "gemini/"}"#)]
#[case::unprefixed_model(Some("gemini-2.5-flash"), None, r#"{"model": "gemini-2.5-flash"}"#)]
fn model_over_agent(#[case] model: Option<&str>, #[case] agent: Option<&str>, #[case] want: &str) {
    assert_eq!(
        built(model, agent, &InteractionOptions::default()),
        parse(want)
    );
}

#[rstest]
#[case::neither(None, None)]
#[case::both_empty(Some(""), Some(""))]
#[case::empty_agent_only(None, Some(""))]
fn a_target_is_required(#[case] model: Option<&str>, #[case] agent: Option<&str>) {
    let error = transform_request(model, agent, None, &InteractionOptions::default()).unwrap_err();
    assert!(matches!(error, StudioInteractionError::MissingTarget));
}

#[test]
fn false_and_empty_optional_values_are_kept() {
    let given = options(
        r#"{"tools": [], "system_instruction": "", "stream": false, "store": false,
            "background": false, "environment": "", "response_modalities": [],
            "previous_interaction_id": ""}"#,
    );
    let input = Json::Text(String::new());
    let request = transform_request(Some("m"), None, Some(&input), &given).unwrap();
    assert_eq!(
        dumped(&request),
        parse(
            r#"{"model": "m", "input": "", "tools": [], "system_instruction": "", "stream": false,
                "store": false, "background": false, "environment": "",
                "response_modalities": [], "previous_interaction_id": ""}"#
        )
    );
}

#[test]
fn present_optional_values_are_copied() {
    let given = options(
        r#"{"tools": [{"google_search": {}}], "system_instruction": "be brief", "stream": true,
            "store": true, "background": true, "environment": {"os": "linux"},
            "response_modalities": ["text", "image"], "previous_interaction_id": "v1_p"}"#,
    );
    let input = parse(r#"[{"role": "user"}]"#);
    let request = transform_request(None, Some("agent"), Some(&input), &given).unwrap();
    assert_eq!(
        dumped(&request),
        parse(
            r#"{"agent": "agent", "input": [{"role": "user"}], "tools": [{"google_search": {}}],
                "system_instruction": "be brief", "stream": true, "store": true,
                "background": true, "environment": {"os": "linux"},
                "response_modalities": ["text", "image"], "previous_interaction_id": "v1_p"}"#
        )
    );
}

#[test]
fn absent_optional_values_and_a_null_input_are_omitted() {
    let request = transform_request(
        Some("m"),
        None,
        Some(&Json::Null),
        &InteractionOptions::default(),
    )
    .unwrap();
    assert_eq!(dumped(&request), parse(r#"{"model": "m"}"#));
}

#[rstest]
#[case::bare_mime(
    r#"{"response_mime_type": "application/json"}"#,
    r#"{"type": "text", "mime_type": "application/json"}"#
)]
#[case::schema_object(
    r#"{"response_mime_type": "application/json", "response_format": {"type": "object"}}"#,
    r#"{"type": "text", "mime_type": "application/json", "schema": {"type": "object"}}"#
)]
#[case::schema_text(
    r#"{"response_mime_type": "text/x", "response_format": "json"}"#,
    r#"{"type": "text", "mime_type": "text/x", "schema": "json"}"#
)]
#[case::list_suppresses(
    r#"{"response_mime_type": "application/json", "response_format": []}"#,
    "[]"
)]
#[case::list_with_entries(
    r#"{"response_mime_type": "application/json", "response_format": [{"type": "text"}]}"#,
    r#"[{"type": "text"}]"#
)]
#[case::mime_in_object_suppresses(
    r#"{"response_mime_type": "application/json", "response_format": {"mime_type": "text/plain"}}"#,
    r#"{"mime_type": "text/plain"}"#
)]
#[case::format_without_mime_type(
    r#"{"response_format": {"type": "object"}}"#,
    r#"{"type": "object"}"#
)]
fn mime_wrap_schema(#[case] given: &str, #[case] want: &str) {
    let body = built(Some("m"), None, &options(given));
    let Json::Object(fields) = body else {
        panic!("the request is an object");
    };
    assert_eq!(fields.get("response_format"), Some(&parse(want)));
}

#[test]
fn an_empty_mime_type_does_not_wrap() {
    let kept = built(
        Some("m"),
        None,
        &options(r#"{"response_mime_type": "", "response_format": "json"}"#),
    );
    assert_eq!(kept, parse(r#"{"model": "m", "response_format": "json"}"#));
    let omitted = built(Some("m"), None, &options(r#"{"response_mime_type": ""}"#));
    assert_eq!(omitted, parse(r#"{"model": "m"}"#));
}

#[rstest]
#[case::alone(
    r#"{"generation_config": {"image_config": {"aspect_ratio": "1:1"}}}"#,
    r#"{"model": "m", "response_format": {"type": "image", "aspect_ratio": "1:1"}}"#
)]
#[case::other_keys_stay(
    r#"{"generation_config": {"temperature": "warm", "image_config": {"aspect_ratio": "1:1"}}}"#,
    r#"{"model": "m", "generation_config": {"temperature": "warm"},
        "response_format": {"type": "image", "aspect_ratio": "1:1"}}"#
)]
#[case::appended_to_list(
    r#"{"response_format": [{"type": "text"}], "generation_config": {"image_config": {"size": "2K"}}}"#,
    r#"{"model": "m", "response_format": [{"type": "text"}, {"type": "image", "size": "2K"}]}"#
)]
#[case::scalar_becomes_pair(
    r#"{"response_format": "json", "generation_config": {"image_config": {"size": "2K"}}}"#,
    r#"{"model": "m", "response_format": ["json", {"type": "image", "size": "2K"}]}"#
)]
#[case::wrapped_mime_becomes_pair(
    r#"{"response_mime_type": "application/json", "generation_config": {"image_config": {}}}"#,
    r#"{"model": "m", "response_format": [{"type": "text", "mime_type": "application/json"}, {"type": "image"}]}"#
)]
#[case::empty_image_config_still_moves(
    r#"{"generation_config": {"image_config": {}}}"#,
    r#"{"model": "m", "response_format": {"type": "image"}}"#
)]
#[case::null_image_config_is_absent(
    r#"{"generation_config": {"image_config": null, "top_k": "3"}}"#,
    r#"{"model": "m", "generation_config": {"top_k": "3"}}"#
)]
#[case::empty_generation_config_is_omitted(r#"{"generation_config": {}}"#, r#"{"model": "m"}"#)]
fn image_format_append(#[case] given: &str, #[case] want: &str) {
    assert_eq!(built(Some("m"), None, &options(given)), parse(want));
}

#[test]
fn the_image_config_type_overwrites_the_initial_type() {
    let given =
        options(r#"{"generation_config": {"image_config": {"type": "custom", "size": "1K"}}}"#);
    let request = transform_request(Some("m"), None, None, &given).unwrap();
    let text = serde_json::to_string(&request).unwrap();
    assert!(
        text.contains(r#""response_format":{"type":"custom","size":"1K"}"#),
        "{text}"
    );
}

#[rstest]
#[case::text(r#"{"generation_config": {"image_config": "1:1"}}"#)]
#[case::list(r#"{"generation_config": {"image_config": []}}"#)]
fn an_image_config_that_is_not_an_object_is_refused(#[case] given: &str) {
    let error = transform_request(Some("m"), None, None, &options(given)).unwrap_err();
    assert!(matches!(
        error,
        StudioInteractionError::ImageConfigNotObject
    ));
}

#[test]
fn the_caller_options_stay_unchanged() {
    let given = options(
        r#"{"response_mime_type": "application/json", "response_format": {"type": "object"},
            "generation_config": {"temperature": "warm", "image_config": {"size": "1K"}}}"#,
    );
    let before = given.clone();
    transform_request(Some("m"), None, None, &given).unwrap();
    assert_eq!(given, before);
}

#[rstest]
#[case::no_params(None, None, false)]
#[case::params_without_key(Some(params(None)), None, false)]
#[case::params_with_key(Some(params(Some("param-key"))), None, true)]
#[case::params_with_environment_key(Some(params(None)), Some("env-key"), true)]
fn the_key_header_needs_params(
    #[case] given: Option<StudioParams>,
    #[case] env_key: Option<&str>,
    #[case] present: bool,
) {
    let headers = validate_environment(&IndexMap::new(), given.as_ref(), &settings(env_key));
    assert_eq!(headers.contains_key("x-goog-api-key"), present);
}

#[test]
fn no_params_never_read_the_environment_key() {
    let headers = validate_environment(&IndexMap::new(), None, &settings(Some("env-key")));
    assert!(!headers.contains_key("x-goog-api-key"));
}

#[rstest]
#[case::param_first(Some("param-key"), Some("env-key"), "param-key")]
#[case::empty_param_falls_back(Some(""), Some("env-key"), "env-key")]
#[case::environment_only(None, Some("env-key"), "env-key")]
fn the_key_comes_from_params_then_environment(
    #[case] param: Option<&str>,
    #[case] env_key: Option<&str>,
    #[case] want: &str,
) {
    let headers = validate_environment(&IndexMap::new(), Some(&params(param)), &settings(env_key));
    assert_eq!(
        headers
            .get("x-goog-api-key")
            .map(secrecy::ExposeSecret::expose_secret),
        Some(want)
    );
}

#[test]
fn the_revision_and_content_type_are_forced() {
    let caller = headers(&[
        ("Content-Type", "text/plain"),
        ("Api-Revision", "2026-05-07"),
        ("X-Caller", "kept"),
        ("content-type", "other"),
    ]);
    let result = validate_environment(&caller, None, &settings(None));
    let plain: Vec<(&str, &str)> = result
        .iter()
        .map(|(name, value)| (name.as_str(), value.expose_secret()))
        .collect();
    assert_eq!(
        plain,
        [
            ("Content-Type", "application/json"),
            ("Api-Revision", API_REVISION),
            ("X-Caller", "kept"),
            ("content-type", "other"),
        ]
    );
    assert_eq!(caller.len(), 4);
}

#[rstest]
#[case::explicit(Some("https://proxy.test"), "https://proxy.test")]
#[case::empty_uses_default(Some(""), DEFAULT_BASE)]
#[case::absent_uses_default(None, DEFAULT_BASE)]
fn the_api_base_falls_back_to_the_settings(#[case] given: Option<&str>, #[case] want: &str) {
    assert_eq!(resolve_api_base(given, &settings(None)), want);
}

#[rstest]
#[case::plain(false, "https://generativelanguage.googleapis.com/v1beta/interactions")]
#[case::stream(
    true,
    "https://generativelanguage.googleapis.com/v1beta/interactions?alt=sse"
)]
fn create_urls_follow_stream(#[case] stream: bool, #[case] want: &str) {
    let url = create_url(None, &params(Some("k")), stream, &settings(None)).unwrap();
    assert_eq!(url, want);
}

#[test]
fn create_urls_use_the_given_base_and_the_environment_key() {
    let url = create_url(
        Some("http://localhost:9"),
        &params(None),
        false,
        &settings(Some("e")),
    );
    assert_eq!(url.unwrap(), "http://localhost:9/v1beta/interactions");
}

#[test]
fn the_create_url_needs_a_key() {
    let create = create_url(None, &params(Some("")), true, &settings(None)).unwrap_err();
    assert!(matches!(create, StudioInteractionError::MissingApiKey));
}

#[rstest]
#[case::get(Lifecycle::Get)]
#[case::delete(Lifecycle::Delete)]
#[case::cancel(Lifecycle::Cancel)]
fn every_lifecycle_url_needs_a_key(#[case] action: Lifecycle) {
    let error = lifecycle_request(action, "id", None, &params(None), &settings(None));
    assert!(matches!(
        error.unwrap_err(),
        StudioInteractionError::MissingApiKey
    ));
}

#[rstest]
#[case::get(Lifecycle::Get, "v1_a", "/v1beta/interactions/v1_a")]
#[case::delete(Lifecycle::Delete, "v1_a", "/v1beta/interactions/v1_a")]
#[case::cancel(Lifecycle::Cancel, "v1_a", "/v1beta/interactions/v1_a:cancel")]
#[case::encoded_cancel(
    Lifecycle::Cancel,
    "a/b c?d#e%",
    "/v1beta/interactions/a%2Fb%20c%3Fd%23e%25:cancel"
)]
#[case::unreserved_kept(Lifecycle::Get, "a-b.c_d~e", "/v1beta/interactions/a-b.c_d~e")]
#[case::utf8(Lifecycle::Get, "\u{e9}", "/v1beta/interactions/%C3%A9")]
fn lifecycle_urls_encode_one_segment(
    #[case] action: Lifecycle,
    #[case] interaction_id: &str,
    #[case] tail: &str,
) {
    let url = lifecycle_request(
        action,
        interaction_id,
        None,
        &params(Some("k")),
        &settings(None),
    )
    .unwrap();
    assert_eq!(url, format!("{DEFAULT_BASE}{tail}"));
}

#[rstest]
#[case::empty("")]
#[case::dot(".")]
#[case::dot_dot("..")]
fn bad_interaction_ids_are_refused(#[case] interaction_id: &str) {
    let error = lifecycle_request(
        Lifecycle::Get,
        interaction_id,
        None,
        &params(Some("k")),
        &settings(None),
    )
    .unwrap_err();
    assert!(matches!(error, StudioInteractionError::InteractionId(_)));
}

#[test]
fn create_stores_raw_headers_and_the_projection() {
    let raw = headers(&[("x-ratelimit-remaining-requests", "9")]);
    let response = transform_response(
        &reply(200, &raw, r#"{"id": "v1_a", "status": "completed"}"#),
        marked,
    )
    .unwrap();
    assert_eq!(response.record.id.as_deref(), Some("v1_a"));
    assert_eq!(response.hidden_params.headers, raw);
    assert_eq!(
        response.hidden_params.additional_headers,
        Some(marked(&raw))
    );
}

#[test]
fn get_stores_raw_headers_only() {
    let raw = headers(&[("x-a", "1")]);
    let response = transform_get_response(&reply(200, &raw, r#"{"id": "v1_a"}"#)).unwrap();
    assert_eq!(response.hidden_params.headers, raw);
    assert_eq!(response.hidden_params.additional_headers, None);
}

#[test]
fn a_sparse_record_keeps_only_the_object_default() {
    let raw = IndexMap::new();
    let record = transform_get_response(&reply(200, &raw, "{}"))
        .unwrap()
        .record;
    assert_eq!(
        dumped(&record),
        parse(
            r#"{"id": null, "object": "interaction", "model": null, "agent": null,
                "status": null, "created": null, "updated": null, "outputs": null,
                "steps": null, "usage": null}"#
        )
    );
}

#[test]
fn an_explicit_null_object_stays_null() {
    let raw = IndexMap::new();
    let record = transform_get_response(&reply(200, &raw, r#"{"object": null}"#))
        .unwrap()
        .record;
    assert_eq!(record.object, None);
}

#[test]
fn native_content_and_unknown_fields_travel_in_order() {
    let raw = IndexMap::new();
    let body = r#"{"zeta": 1, "id": "v1_a", "steps": [{"type": "text", "n": 2.5}],
                   "usage": {"total_tokens": 12}, "alpha": {"deep": [true, null, 18446744073709551615]}}"#;
    let record = transform_get_response(&reply(200, &raw, body))
        .unwrap()
        .record;
    let text = serde_json::to_string(&record).unwrap();
    assert!(
        text.contains(r#""steps":[{"type":"text","n":2.5}]"#),
        "{text}"
    );
    assert!(text.contains(r#""usage":{"total_tokens":12}"#), "{text}");
    assert!(
        text.ends_with(r#""zeta":1,"alpha":{"deep":[true,null,18446744073709551615]}}"#),
        "{text}"
    );
    assert_eq!(record.extension.len(), 2);
}

#[rstest]
#[case::ok(200)]
#[case::bad_gateway(502)]
fn malformed_json_becomes_a_gemini_error_with_the_reply(#[case] status: u16) {
    let raw = headers(&[("content-type", "text/html")]);
    let error = transform_response(&reply(status, &raw, "<html>bad"), marked).unwrap_err();
    let StudioInteractionError::Gemini {
        status: got,
        body,
        headers,
    } = error
    else {
        panic!("expected a Gemini error");
    };
    assert_eq!((got, body.as_str(), &headers), (status, "<html>bad", &raw));
}

#[test]
fn a_reply_that_is_not_text_is_reported_by_size() {
    let raw = IndexMap::new();
    let response = RawResponse {
        status: 500,
        headers: &raw,
        body: &[0xff, 0xfe, 0xfd],
    };
    let StudioInteractionError::Gemini { body, .. } =
        transform_get_response(&response).unwrap_err()
    else {
        panic!("expected a Gemini error");
    };
    assert_eq!(body, "3 bytes that are not UTF-8");
}

#[test]
fn a_truncated_record_is_malformed_json_not_a_record_error() {
    let raw = IndexMap::new();
    let error = transform_get_response(&reply(200, &raw, r#"{"id": 5, "x""#)).unwrap_err();
    assert!(matches!(error, StudioInteractionError::Gemini { .. }));
}

#[rstest]
#[case::array("[1]")]
#[case::null("null")]
#[case::text(r#""x""#)]
#[case::id_number(r#"{"id": 5}"#)]
#[case::steps_object(r#"{"steps": {"a": 1}}"#)]
#[case::steps_items(r#"{"steps": [1]}"#)]
#[case::usage_list(r#"{"usage": []}"#)]
fn valid_json_of_the_wrong_shape_is_a_record_error(#[case] body: &str) {
    let raw = IndexMap::new();
    let error = transform_get_response(&reply(200, &raw, body)).unwrap_err();
    assert!(matches!(error, StudioInteractionError::Record(_)));
}

#[test]
fn the_response_limit_is_inclusive() {
    let raw = IndexMap::new();
    let padding = " ".repeat(RESPONSE_BODY_LIMIT - 2);
    let at_limit = format!("{{}}{padding}");
    transform_get_response(&reply(200, &raw, &at_limit)).unwrap();
    let over = format!("{at_limit} ");
    let error = transform_get_response(&reply(200, &raw, &over)).unwrap_err();
    assert!(matches!(
        error,
        StudioInteractionError::ResponseTooLarge { size, limit }
            if size == RESPONSE_BODY_LIMIT + 1 && limit == RESPONSE_BODY_LIMIT
    ));
}

#[test]
fn cancel_keeps_its_two_fields_and_the_rest() {
    let raw = IndexMap::new();
    let body = r#"{"id": "v1_a", "status": "cancelled", "x": 1}"#;
    let result =
        decode_studio_interaction::<CancelInteractionResult>(&reply(200, &raw, body)).unwrap();
    assert_eq!(result.id.as_deref(), Some("v1_a"));
    assert_eq!(result.status.as_deref(), Some("cancelled"));
    assert_eq!(result.extension.len(), 1);
    let error =
        decode_studio_interaction::<CancelInteractionResult>(&reply(200, &raw, "no")).unwrap_err();
    assert!(matches!(error, StudioInteractionError::Gemini { .. }));
}

#[rstest]
#[case::no_content(204, "")]
#[case::ok_with_junk(200, "not json")]
#[case::last_success(299, "<html>")]
#[case::first_success(200, "{}")]
fn delete_2xx_succeeds_without_reading_the_body(#[case] status: u16, #[case] body: &str) {
    let raw = IndexMap::new();
    let result = transform_delete_response(&reply(status, &raw, body), "v1_a").unwrap();
    assert!(result.success);
    assert_eq!(result.id, "v1_a");
}

#[rstest]
#[case::below(199)]
#[case::above(300)]
#[case::not_found(404)]
fn delete_outside_2xx_is_a_gemini_error(#[case] status: u16) {
    let raw = headers(&[("x-h", "v")]);
    let error = transform_delete_response(&reply(status, &raw, "gone"), "v1_a").unwrap_err();
    let StudioInteractionError::Gemini {
        status: got,
        body,
        headers,
    } = error
    else {
        panic!("expected a Gemini error");
    };
    assert_eq!((got, body.as_str(), &headers), (status, "gone", &raw));
}

#[test]
fn a_streaming_chunk_keeps_sparse_fields() {
    let chunk =
        parse(r#"{"event_type": "step.delta", "index": 3, "delta": {"text": "a"}, "extra": 7}"#);
    let record = transform_streaming_response(&chunk).unwrap();
    assert_eq!(
        dumped(&record),
        parse(
            r#"{"event_type": "step.delta", "id": null, "object": "interaction", "model": null,
                "agent": null, "status": null, "created": null, "updated": null,
                "outputs": null, "steps": null, "usage": null, "delta": {"text": "a"},
                "index": 3, "step": null, "interaction_id": null, "interaction": null,
                "extra": 7}"#
        )
    );
    let empty = transform_streaming_response(&parse("{}")).unwrap();
    assert_eq!(empty.object.as_deref(), Some("interaction"));
    assert_eq!(empty.event_type, None);
}

#[test]
fn a_streaming_chunk_must_be_an_object() {
    let error = transform_streaming_response(&parse("[]")).unwrap_err();
    assert!(matches!(error, StudioInteractionError::Record(_)));
}

#[rstest]
fn replays_the_studio_interaction_corpus(
    #[files("fuzz/corpus/studio_interaction/*")] path: PathBuf,
) {
    let raw = IndexMap::new();
    let bytes = fs::read(&path).expect("seed bytes");
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("utf-8 name");
    let response = RawResponse {
        status: 200,
        headers: &raw,
        body: &bytes,
    };
    let interaction = decode_studio_interaction::<InteractionRecord>(&response);
    let cancel = decode_studio_interaction::<CancelInteractionResult>(&response);
    assert_eq!(interaction.is_ok(), name.starts_with("valid"), "{name}");
    assert!(cancel.is_ok() || !name.starts_with("valid"), "{name}");
}

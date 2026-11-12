#![cfg(test)]

use rstest::rstest;
use std::collections::{BTreeMap, BTreeSet};

use super::{
    BodyKind, MediaTypeKind, ParsedBody, RequestBodyError, RequestBodyInput, RequestBodyResult,
    RewrittenBody, classify_media_type, is_opaque_audio_request, is_otlp_trace_request,
    normalize_media_type, parse_binary_body, rewrite_request_model,
};
use crate::api::api_boundaries::ApiLimitError;

fn object(source: &str) -> super::JsonObject {
    serde_json::from_str(source).unwrap()
}

fn json_input(body: &str) -> RequestBodyInput {
    RequestBodyInput {
        method: "POST".to_owned(),
        route_path: "/v1/chat/completions".to_owned(),
        headers: BTreeMap::from([("content-type".to_owned(), "application/json".to_owned())]),
        query_params: BTreeMap::new(),
        body: body.as_bytes().to_vec(),
        client_requested_model: None,
        cached_body: None,
        form_fields: None,
    }
}

#[rstest]
#[case::bare("application/json", "application/json")]
#[case::parameters("application/json; charset=utf-8", "application/json")]
#[case::upper("Application/JSON", "application/json")]
#[case::spaces(" text/plain ; q=1 ", "text/plain")]
#[case::empty("", "")]
#[case::only_parameters("; charset=utf-8", "")]
fn normalize_media_type_strips_parameters(#[case] content_type: &str, #[case] expected: &str) {
    assert_eq!(normalize_media_type(content_type), expected);
}

#[rstest]
#[case::json("application/json", MediaTypeKind::Json)]
#[case::json_parameters("application/json; charset=utf-8", MediaTypeKind::Json)]
#[case::json_upper("APPLICATION/JSON", MediaTypeKind::Json)]
#[case::urlencoded("application/x-www-form-urlencoded", MediaTypeKind::Form)]
#[case::multipart("multipart/form-data; boundary=x", MediaTypeKind::Form)]
#[case::protobuf("application/x-protobuf", MediaTypeKind::Binary)]
#[case::protobuf_short("application/protobuf", MediaTypeKind::Binary)]
#[case::text("text/plain", MediaTypeKind::Other)]
#[case::absent("", MediaTypeKind::Other)]
fn classify_media_type_names_the_branch(
    #[case] content_type: &str,
    #[case] expected: MediaTypeKind,
) {
    assert_eq!(classify_media_type(content_type), expected);
}

#[rstest]
#[case::object(r#"{"model":"gpt-4o"}"#, true)]
#[case::array("[1,2]", false)]
#[case::string(r#""hi""#, false)]
#[case::invalid("{\"model\":", false)]
#[case::empty("", false)]
fn parse_binary_body_keeps_only_objects(#[case] body: &str, #[case] is_object: bool) {
    let parsed = parse_binary_body(body.as_bytes());
    assert_eq!(parsed.contains_key("model"), is_object);
}

#[rstest]
#[case::post_traces("POST", "/v1/traces", true)]
#[case::get_traces("GET", "/v1/traces", false)]
#[case::other_method("PUT", "/v1/traces", false)]
#[case::trailing_slash("POST", "/v1/traces/", false)]
#[case::other_path("POST", "/v1/chat/completions", false)]
fn otlp_trace_requests_are_posted_to_the_traces_path(
    #[case] method: &str,
    #[case] route_path: &str,
    #[case] expected: bool,
) {
    assert_eq!(is_otlp_trace_request(method, route_path), expected);
}

#[rstest]
#[case::raw_audio("/azure_speech/audio/speech", "audio/wav", true)]
#[case::parameterized_audio("/azure_speech/audio/speech", "AUDIO/WAV; codecs=1", true)]
#[case::multipart(
    "/azure_speech/audio/transcriptions",
    "multipart/form-data; boundary=x",
    true
)]
#[case::bare_prefix("/azure_speech", "audio/wav", false)]
#[case::other_route("/v1/audio/speech", "audio/wav", false)]
#[case::json_body("/azure_speech/audio/speech", "application/json", false)]
fn opaque_audio_needs_the_azure_speech_route_and_audio_bytes(
    #[case] route: &str,
    #[case] content_type: &str,
    #[case] expected: bool,
) {
    assert_eq!(is_opaque_audio_request(route, content_type), expected);
}

#[test]
fn cached_body_hides_keys_merged_after_the_cache_was_set() {
    let cached = ParsedBody {
        accepted_keys: BTreeSet::from(["model".to_owned()]),
        body: object(r#"{"model":"gpt-4o","api_key":"merged"}"#),
    };
    assert_eq!(cached.visible(), object(r#"{"model":"gpt-4o"}"#));
}

#[test]
fn rewrite_request_model_points_the_cache_at_the_new_model() {
    let input = RequestBodyInput {
        cached_body: Some(ParsedBody::from_client_body(object(
            r#"{"model":"old","messages":[]}"#,
        ))),
        ..json_input(r#"{"model":"old","api_key":"merged"}"#)
    };
    let rewritten = rewrite_request_model(&object(r#"{"model":"old"}"#), Some(&input), "new");
    assert_eq!(rewritten.request_data, object(r#"{"model":"new"}"#));
    assert_eq!(
        rewritten.cached_body.as_ref().map(ParsedBody::visible),
        Some(object(r#"{"model":"new","messages":[]}"#))
    );
    let raw = rewritten.raw_json_body.unwrap();
    assert_eq!(
        serde_json::from_slice::<super::JsonObject>(&raw).unwrap(),
        object(r#"{"model":"new","messages":[]}"#)
    );
}

#[test]
fn rewrite_request_model_without_a_request_changes_nothing_else() {
    let rewritten = rewrite_request_model(&object(r#"{"model":"old"}"#), None, "new");
    assert_eq!(
        rewritten,
        RewrittenBody {
            request_data: object(r#"{"model":"new"}"#),
            cached_body: None,
            raw_json_body: None,
        }
    );
}

#[test]
fn rewrite_request_model_without_a_cache_caches_the_whole_payload() {
    let input = json_input(r#"{"model":"old"}"#);
    let rewritten = rewrite_request_model(
        &object(r#"{"model":"old","api_key":"merged"}"#),
        Some(&input),
        "new",
    );
    assert_eq!(
        rewritten.cached_body.as_ref().map(ParsedBody::visible),
        Some(object(r#"{"model":"new","api_key":"merged"}"#))
    );
}

#[rstest]
#[case::invalid_json(RequestBodyError::InvalidJson)]
#[case::not_an_object(RequestBodyError::NotAnObject)]
#[case::undecoded_form(RequestBodyError::UndecodedForm)]
#[case::invalid_metadata(RequestBodyError::InvalidFormMetadata)]
fn body_errors_are_request_body_four_hundreds(#[case] error: RequestBodyError) {
    let facts = error.facts();
    assert_eq!(facts.status, 400);
    assert_eq!(facts.error_type, "invalid_request_error");
    assert_eq!(facts.param, "request_body");
    assert_eq!(facts.code, "400");
}

#[test]
fn a_limit_error_keeps_the_boundary_facts() {
    let error = RequestBodyError::from(ApiLimitError::JsonTooLarge {
        observed: 20,
        limit: 16,
    });
    let facts = error.facts();
    assert_eq!(facts.status, 413);
    assert_eq!(facts.param, "body");
    assert_eq!(facts.code, "request_too_large");
}

#[test]
fn an_unread_request_has_no_body_to_expose() {
    let input = json_input(r#"{"model":"gpt-4o"}"#);
    let result = RequestBodyResult {
        branch: BodyKind::Json,
        body: super::JsonObject::new(),
        body_read: true,
        parsed_body: None,
    };
    assert!(result.cached_body().is_none());
    assert!(result.raw_json_body(&input).is_none());
}

#![cfg(test)]

use betterlite_models::identity::provider_id::ProviderId;
use betterlite_prompts::arguments::json::{Json, Object};
use rstest::rstest;
use secrecy::SecretString;

use super::delete::transform_delete_response;
use super::error::FireworksResponsesError;
use super::headers::validate_environment;
use super::instructions::{fold_instructions, text_only_content};
use super::request::transform_responses_request;
use super::settings::{FireworksResponsesEnv, SessionSource};
use super::url::{DEFAULT_API_BASE, get_complete_url};
use super::{PROVIDER, SUPPORTS_NATIVE_FILE_SEARCH, SUPPORTS_NATIVE_WEBSOCKET};
use crate::fireworks_ai_common::error::MissingApiKey;
use crate::fireworks_ai_common::{FireworksEnv, SessionMetadata};
use crate::openai_base_config::Headers;
use crate::response_input::ResponsesInput;

fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn responses_input(text: &str) -> ResponsesInput {
    serde_json::from_str(text).unwrap()
}

fn params(text: &str) -> Object {
    serde_json::from_str(text).unwrap()
}

fn items_json(input: &ResponsesInput) -> Json {
    match input {
        ResponsesInput::Text(text) => Json::Text(text.clone()),
        ResponsesInput::Items(items) => {
            Json::List(items.iter().map(|item| item.0.clone()).collect())
        }
    }
}

fn folded(input: &str, instructions: Option<&str>) -> (Option<Json>, Json) {
    let (folded, kept) = fold_instructions(&responses_input(input), instructions);
    (folded, items_json(&kept))
}

fn transformed(input: &str, optional: &str) -> Object {
    transform_responses_request("model", &responses_input(input), &params(optional), None).unwrap()
}

fn key(text: &str) -> SecretString {
    SecretString::from(text)
}

fn env_with(
    fireworks: Option<&str>,
    ai: Option<&str>,
    joined: Option<&str>,
    token: Option<&str>,
    api_base: Option<&str>,
) -> FireworksResponsesEnv {
    FireworksResponsesEnv {
        keys: FireworksEnv {
            fireworks_api_key: fireworks.map(key),
            fireworks_ai_api_key: ai.map(key),
            fireworksai_api_key: joined.map(key),
            fireworks_ai_token: token.map(key),
        },
        api_base: api_base.map(str::to_owned),
    }
}

fn headers(pairs: &[(&str, &str)]) -> Headers {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn session(litellm: Option<&str>, metadata: Option<SessionMetadata>) -> SessionSource {
    SessionSource {
        litellm_session_id: litellm.map(str::to_owned),
        litellm_metadata: metadata,
    }
}

fn metadata(session_id: Option<&str>, generated: Option<bool>) -> SessionMetadata {
    SessionMetadata {
        litellm_session_id_generated: generated,
        session_id: session_id.map(str::to_owned),
    }
}

#[test]
fn the_config_flags_name_fireworks_without_native_transports() {
    assert_eq!(PROVIDER, ProviderId::FireworksAi);
    assert!(!SUPPORTS_NATIVE_WEBSOCKET);
    assert!(!SUPPORTS_NATIVE_FILE_SEARCH);
}

#[test]
fn the_headers_start_with_the_json_content_type_then_the_callers_then_bearer() {
    let authorized = validate_environment(
        &headers(&[("X-Test", "1")]),
        Some(&key("arg-key")),
        &session(None, None),
        &env_with(None, None, None, None, None),
    )
    .unwrap();
    assert_eq!(
        authorized,
        headers(&[
            ("Authorization", "Bearer arg-key"),
            ("Content-Type", "application/json"),
            ("X-Test", "1"),
        ])
    );
}

#[test]
fn caller_headers_override_the_content_type_but_never_the_bearer() {
    let authorized = validate_environment(
        &headers(&[
            ("Content-Type", "text/plain"),
            ("Authorization", "Bearer caller"),
        ]),
        Some(&key("arg-key")),
        &session(None, None),
        &env_with(None, None, None, None, None),
    )
    .unwrap();
    assert_eq!(
        authorized,
        headers(&[
            ("Authorization", "Bearer arg-key"),
            ("Content-Type", "text/plain"),
        ])
    );
    let lowercase = validate_environment(
        &headers(&[("content-type", "text/plain")]),
        Some(&key("arg-key")),
        &session(None, None),
        &env_with(None, None, None, None, None),
    )
    .unwrap();
    assert_eq!(
        lowercase,
        headers(&[
            ("Authorization", "Bearer arg-key"),
            ("Content-Type", "application/json"),
            ("content-type", "text/plain"),
        ])
    );
}

#[test]
fn the_env_key_chain_applies_when_the_argument_is_absent_or_empty() {
    let all = env_with(
        Some("first"),
        Some("second"),
        Some("third"),
        Some("fourth"),
        None,
    );
    let resolved = validate_environment(&Headers::new(), None, &session(None, None), &all).unwrap();
    assert_eq!(
        resolved.get("Authorization").map(String::as_str),
        Some("Bearer first")
    );
    let empty =
        validate_environment(&Headers::new(), Some(&key("")), &session(None, None), &all).unwrap();
    assert_eq!(
        empty.get("Authorization").map(String::as_str),
        Some("Bearer first")
    );
    let second = env_with(None, Some("second"), None, None, None);
    let resolved =
        validate_environment(&Headers::new(), None, &session(None, None), &second).unwrap();
    assert_eq!(
        resolved.get("Authorization").map(String::as_str),
        Some("Bearer second")
    );
    let third = env_with(None, None, Some("third"), None, None);
    let resolved =
        validate_environment(&Headers::new(), None, &session(None, None), &third).unwrap();
    assert_eq!(
        resolved.get("Authorization").map(String::as_str),
        Some("Bearer third")
    );
    let fourth = env_with(None, None, None, Some("fourth"), None);
    let resolved =
        validate_environment(&Headers::new(), None, &session(None, None), &fourth).unwrap();
    assert_eq!(
        resolved.get("Authorization").map(String::as_str),
        Some("Bearer fourth")
    );
}

#[test]
fn an_empty_env_key_counts_as_unset() {
    let env = env_with(Some(""), None, None, None, None);
    let error =
        validate_environment(&Headers::new(), None, &session(None, None), &env).unwrap_err();
    assert!(matches!(error, MissingApiKey));
}

#[test]
fn no_key_anywhere_fails_with_the_missing_key_error() {
    let error = validate_environment(
        &Headers::new(),
        None,
        &session(None, None),
        &env_with(None, None, None, None, None),
    )
    .unwrap_err();
    assert!(matches!(error, MissingApiKey));
    assert_eq!(error.to_string(), "FIREWORKS_API_KEY is not set");
}

#[test]
fn a_session_id_pins_the_request_to_one_node() {
    let pinned = validate_environment(
        &Headers::new(),
        Some(&key("arg-key")),
        &session(Some("sess-1"), None),
        &env_with(None, None, None, None, None),
    )
    .unwrap();
    assert_eq!(
        pinned.get("x-session-affinity").map(String::as_str),
        Some("sess-1")
    );
}

#[test]
fn a_metadata_session_id_pins_unless_the_proxy_generated_it() {
    let kept = validate_environment(
        &Headers::new(),
        Some(&key("arg-key")),
        &session(None, Some(metadata(Some("meta-1"), None))),
        &env_with(None, None, None, None, None),
    )
    .unwrap();
    assert_eq!(
        kept.get("x-session-affinity").map(String::as_str),
        Some("meta-1")
    );
    let not_generated = validate_environment(
        &Headers::new(),
        Some(&key("arg-key")),
        &session(None, Some(metadata(Some("meta-1"), Some(false)))),
        &env_with(None, None, None, None, None),
    )
    .unwrap();
    assert_eq!(
        not_generated.get("x-session-affinity").map(String::as_str),
        Some("meta-1")
    );
    let generated = validate_environment(
        &Headers::new(),
        Some(&key("arg-key")),
        &session(None, Some(metadata(Some("meta-1"), Some(true)))),
        &env_with(None, None, None, None, None),
    )
    .unwrap();
    assert_eq!(generated.get("x-session-affinity"), None);
}

#[test]
fn a_callers_session_affinity_is_never_overwritten() {
    let supplied = validate_environment(
        &headers(&[("x-session-affinity", "caller")]),
        Some(&key("arg-key")),
        &session(Some("sess-1"), None),
        &env_with(None, None, None, None, None),
    )
    .unwrap();
    assert_eq!(
        supplied.get("x-session-affinity").map(String::as_str),
        Some("caller")
    );
    let uppercase = validate_environment(
        &headers(&[("X-Session-Affinity", "caller")]),
        Some(&key("arg-key")),
        &session(Some("sess-1"), None),
        &env_with(None, None, None, None, None),
    )
    .unwrap();
    assert_eq!(
        uppercase.get("X-Session-Affinity").map(String::as_str),
        Some("caller")
    );
    assert_eq!(uppercase.get("x-session-affinity"), None);
}

#[test]
fn the_complete_url_takes_the_argument_then_the_env_then_the_default() {
    assert_eq!(
        get_complete_url(None, &env_with(None, None, None, None, None)),
        "https://api.fireworks.ai/inference/v1/responses"
    );
    assert_eq!(
        get_complete_url(
            None,
            &env_with(None, None, None, None, Some("https://env.example/v1/"))
        ),
        "https://env.example/v1/responses"
    );
    assert_eq!(
        get_complete_url(
            Some("https://arg.example/v1///"),
            &env_with(None, None, None, None, Some("https://env.example/v1/")),
        ),
        "https://arg.example/v1/responses"
    );
    assert_eq!(
        get_complete_url(
            Some(""),
            &env_with(None, None, None, None, Some("https://env.example/v1")),
        ),
        "https://env.example/v1/responses"
    );
    assert_eq!(
        get_complete_url(None, &env_with(None, None, None, None, Some(""))),
        "https://api.fireworks.ai/inference/v1/responses"
    );
    assert_eq!(
        get_complete_url(
            Some("http://host/base/"),
            &env_with(None, None, None, None, None)
        ),
        "http://host/base/responses"
    );
    assert_eq!(DEFAULT_API_BASE, "https://api.fireworks.ai/inference/v1");
}

#[test]
fn a_string_input_keeps_its_instructions_as_given() {
    assert_eq!(
        folded(r#""hello""#, Some("sys")),
        (Some(json(r#""sys""#)), json(r#""hello""#))
    );
    assert_eq!(folded(r#""hello""#, None), (None, json(r#""hello""#)));
    assert_eq!(
        folded(r#""hello""#, Some("")),
        (Some(Json::Text(String::new())), json(r#""hello""#))
    );
}

#[test]
fn a_leading_instruction_item_is_folded_after_the_instruction() {
    assert_eq!(
        folded(
            r#"[{"role": "system", "content": "a"}, {"role": "user", "content": "u"}]"#,
            Some("i"),
        ),
        (
            Some(json(r#""i\n\na""#)),
            json(r#"[{"role": "user", "content": "u"}]"#)
        )
    );
    assert_eq!(
        folded(
            r#"[{"role": "developer", "content": "d"}, {"role": "user", "content": "u"}]"#,
            None,
        ),
        (
            Some(json(r#""d""#)),
            json(r#"[{"role": "user", "content": "u"}]"#)
        )
    );
}

#[test]
fn a_text_part_list_is_joined_and_empty_texts_are_skipped() {
    assert_eq!(
        folded(
            r#"[{"role": "system", "content": [{"type": "input_text", "text": "a"}, {"type": "input_text", "text": ""}, {"type": "input_text", "text": "b"}]}, {"role": "user", "content": "u"}]"#,
            None,
        ),
        (
            Some(json(r#""a\n\nb""#)),
            json(r#"[{"role": "user", "content": "u"}]"#)
        )
    );
}

#[test]
fn an_empty_content_list_or_string_is_folded_and_leaves_no_instructions() {
    assert_eq!(
        folded(
            r#"[{"role": "system", "content": []}, {"role": "user", "content": "u"}]"#,
            None,
        ),
        (None, json(r#"[{"role": "user", "content": "u"}]"#))
    );
    assert_eq!(
        folded(
            r#"[{"role": "system", "content": ""}, {"role": "user", "content": "u"}]"#,
            None,
        ),
        (None, json(r#"[{"role": "user", "content": "u"}]"#))
    );
}

#[test]
fn a_non_text_part_keeps_the_item_and_renames_a_developer_to_system() {
    assert_eq!(
        folded(
            r#"[{"role": "developer", "content": [{"type": "input_image", "image_url": "x"}], "name": "n", "id": "x"}, {"role": "user", "content": "u"}]"#,
            None,
        ),
        (
            None,
            json(
                r#"[{"role": "system", "content": [{"type": "input_image", "image_url": "x"}], "type": "message"}, {"role": "user", "content": "u"}]"#,
            )
        )
    );
}

#[test]
fn instructions_after_an_assistant_reply_stay_in_the_input() {
    assert_eq!(
        folded(
            r#"[{"role": "user", "content": "u"}, {"role": "assistant", "content": "a"}, {"role": "developer", "content": "d"}]"#,
            None,
        ),
        (
            None,
            json(
                r#"[{"role": "user", "content": "u"}, {"role": "assistant", "content": "a"}, {"role": "system", "content": "d", "type": "message"}]"#,
            )
        )
    );
}

#[test]
fn a_developer_between_conversation_items_is_folded() {
    assert_eq!(
        folded(
            r#"[{"role": "user", "content": "u"}, {"role": "developer", "content": "d"}, {"role": "assistant", "content": "a"}, {"role": "developer", "content": "e"}, {"role": "user", "content": "u2"}]"#,
            None,
        ),
        (
            Some(json(r#""d\n\ne""#)),
            json(
                r#"[{"role": "user", "content": "u"}, {"role": "assistant", "content": "a"}, {"role": "user", "content": "u2"}]"#,
            )
        )
    );
}

#[test]
fn an_assistant_last_item_leaves_everything_alone() {
    assert_eq!(
        folded(
            r#"[{"role": "user", "content": "u"}, {"role": "assistant", "content": "a"}]"#,
            None,
        ),
        (
            None,
            json(r#"[{"role": "user", "content": "u"}, {"role": "assistant", "content": "a"}]"#),
        )
    );
}

#[test]
fn a_system_item_after_the_conversation_starts_is_kept() {
    assert_eq!(
        folded(
            r#"[{"role": "user", "content": "u"}, {"role": "system", "content": "s"}]"#,
            None,
        ),
        (
            None,
            json(r#"[{"role": "user", "content": "u"}, {"role": "system", "content": "s"}]"#),
        )
    );
    assert_eq!(
        folded(
            r#"[{"role": "system", "content": "a"}, {"role": "user", "content": "u"}, {"role": "system", "content": "s"}]"#,
            None,
        ),
        (
            Some(json(r#""a""#)),
            json(r#"[{"role": "user", "content": "u"}, {"role": "system", "content": "s"}]"#),
        )
    );
}

#[test]
fn text_only_content_ignores_a_non_instruction_role() {
    let user = json(r#"{"role": "user", "content": "u"}"#);
    assert_eq!(text_only_content(&user), None);
    let system = json(r#"{"role": "system", "content": "s"}"#);
    assert_eq!(text_only_content(&system), Some(String::from("s")));
}

#[test]
fn an_empty_folded_text_adds_no_separator() {
    assert_eq!(
        folded(
            r#"[{"role": "system", "content": ""}, {"role": "system", "content": "a"}, {"role": "user", "content": "u"}]"#,
            None,
        ),
        (
            Some(json(r#""a""#)),
            json(r#"[{"role": "user", "content": "u"}]"#),
        )
    );
}

#[test]
fn a_non_object_item_passes_through() {
    assert_eq!(
        folded(r#"[{"role": "user", "content": "u"}, "plain", 5]"#, None),
        (
            None,
            json(r#"[{"role": "user", "content": "u"}, "plain", 5]"#),
        )
    );
}

#[test]
fn a_developer_item_without_content_is_kept_as_it_arrived() {
    assert_eq!(
        folded(
            r#"[{"role": "developer"}, {"role": "user", "content": "u"}]"#,
            None,
        ),
        (
            None,
            json(r#"[{"role": "developer"}, {"role": "user", "content": "u"}]"#),
        )
    );
}

#[test]
fn an_empty_instruction_is_skipped_before_the_folded_texts() {
    assert_eq!(
        folded(
            r#"[{"role": "system", "content": "a"}, {"role": "user", "content": "u"}]"#,
            Some(""),
        ),
        (
            Some(json(r#""a""#)),
            json(r#"[{"role": "user", "content": "u"}]"#),
        )
    );
}

#[test]
fn an_empty_instruction_survives_when_nothing_is_folded() {
    assert_eq!(
        folded(r#"[{"role": "user", "content": "u"}]"#, Some("")),
        (
            Some(Json::Text(String::new())),
            json(r#"[{"role": "user", "content": "u"}]"#),
        )
    );
}

#[test]
fn the_transform_folds_instructions_and_resolves_the_model() {
    assert_eq!(
        transformed("hello", "{}"),
        params(r#"{"model": "accounts/fireworks/models/model", "input": "hello"}"#)
    );
    assert_eq!(
        transformed(
            r#"[{"role": "system", "content": "a"}, {"role": "user", "content": "u"}]"#,
            r#"{"instructions": "i"}"#,
        ),
        params(
            r#"{"model": "accounts/fireworks/models/model", "input": [{"role": "user", "content": "u"}], "instructions": "i\n\na"}"#,
        )
    );
    let already = transform_responses_request(
        "accounts/fireworks/models/x",
        &responses_input("hello"),
        &params("{}"),
        None,
    )
    .unwrap();
    assert_eq!(
        already.get("model"),
        Some(&json(r#""accounts/fireworks/models/x""#))
    );
}

#[test]
fn a_null_instruction_disappears_and_the_input_still_folds() {
    let body = transformed(
        r#"[{"role": "system", "content": "a"}, {"role": "user", "content": "u"}]"#,
        r#"{"instructions": null, "temperature": 1}"#,
    );
    assert_eq!(
        body,
        params(
            r#"{"temperature": 1, "instructions": "a", "model": "accounts/fireworks/models/model", "input": [{"role": "user", "content": "u"}]}"#,
        )
    );
    let string = transformed("hello", r#"{"instructions": null}"#);
    assert_eq!(
        string,
        params(r#"{"model": "accounts/fireworks/models/model", "input": "hello"}"#)
    );
}

#[test]
fn a_non_string_instruction_is_kept_and_only_developers_are_renamed() {
    let body = transformed(
        r#"[{"role": "developer", "content": "d"}, {"role": "user", "content": "u"}]"#,
        r#"{"instructions": {"a": 1}}"#,
    );
    assert_eq!(
        body,
        params(
            r#"{"instructions": {"a": 1}, "model": "accounts/fireworks/models/model", "input": [{"role": "system", "content": "d", "type": "message"}, {"role": "user", "content": "u"}]}"#,
        )
    );
    let string = transformed("hello", r#"{"instructions": 5}"#);
    assert_eq!(
        string,
        params(
            r#"{"instructions": 5, "model": "accounts/fireworks/models/model", "input": "hello"}"#,
        )
    );
}

#[test]
fn the_instructions_entry_moves_to_the_end_of_the_params() {
    let body = transformed(
        "hello",
        r#"{"temperature": 1, "instructions": "i", "max_output_tokens": 3}"#,
    );
    assert_eq!(
        body.keys().map(String::as_str).collect::<Vec<&str>>(),
        [
            "temperature",
            "max_output_tokens",
            "instructions",
            "model",
            "input"
        ]
    );
    let removed = transformed("hello", r#"{"temperature": 1, "instructions": null}"#);
    assert_eq!(
        removed.keys().map(String::as_str).collect::<Vec<&str>>(),
        ["temperature", "model", "input"]
    );
}

#[test]
fn a_bad_tools_value_keeps_the_base_transform_error() {
    let error = transform_responses_request(
        "model",
        &responses_input("hello"),
        &params(r#"{"tools": "x"}"#),
        None,
    )
    .unwrap_err();
    assert!(matches!(error, FireworksResponsesError::Input(_)));
    assert_eq!(error.status_code(), Some(400));
}

#[test]
fn a_param_that_fixes_the_body_key_keeps_the_base_error() {
    let error = transform_responses_request(
        "model",
        &responses_input("hello"),
        &params(r#"{"model": "other"}"#),
        None,
    )
    .unwrap_err();
    assert!(matches!(error, FireworksResponsesError::Input(_)));
    assert_eq!(error.status_code(), None);
}

#[rstest]
#[case::plain("/inference/v1/responses/resp_abc", "resp_abc")]
#[case::encoded_slash("/x/a%2Fb", "b")]
#[case::double_encoded("/x/a%252Fb", "a/b")]
#[case::double_encoded_space("/x/a%2520b", "a b")]
#[case::invalid_utf8("/x/%FF", "\u{fffd}")]
#[case::plus("/x/a+b", "a+b")]
#[case::invalid_escape("/x/%zz", "%zz")]
#[case::encoded_letters("/x/%2541", "A")]
#[case::root("/", "")]
#[case::empty("", "")]
#[case::trailing_slash("/x/", "")]
fn the_deleted_id_is_the_decoded_last_segment(#[case] path: &str, #[case] id: &str) {
    let deleted = transform_delete_response(path).unwrap();
    assert_eq!(deleted.id.as_deref(), Some(id));
    assert_eq!(deleted.object.as_deref(), Some("response"));
    assert_eq!(deleted.deleted, Some(true));
    assert!(deleted.extra.is_empty());
}

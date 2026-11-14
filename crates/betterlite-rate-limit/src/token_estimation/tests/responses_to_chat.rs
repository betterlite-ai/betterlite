#![cfg(test)]

use std::sync::{Arc, Mutex};

use betterlite_config::sections::local_ttl_cache::Settings;
use betterlite_context::local_ttl_cache::dual::{Diagnostics, DualLocalCache};
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use rstest::rstest;

use super::input_text::responses_chat_messages;
use super::services::responses::CachedResponsesInput;
use super::support::{RecordingResponses, request, value};

fn empty_cache() -> DualLocalCache {
    let diagnostics = Diagnostics {
        redactor: Arc::new(Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("a redactor")),
        emit: Arc::new(|_event| {}),
    };
    DualLocalCache::new(
        None,
        None,
        &Settings::default(),
        Arc::new(|| 0.0_f64),
        diagnostics,
    )
}

fn converted(body: &str) -> Vec<Json> {
    let cache = empty_cache();
    let converter = CachedResponsesInput {
        cache: &cache,
        limits: ApiLimits::default(),
    };
    responses_chat_messages(&converter, &request(body))
}

#[test]
fn an_absent_input_becomes_one_empty_user_message() {
    assert_eq!(
        converted("{}"),
        vec![value(r#"{"role":"user","content":""}"#)]
    );
}

#[rstest]
#[case::null(r#"{"input":null}"#)]
#[case::empty_text(r#"{"input":""}"#)]
fn a_null_or_empty_input_becomes_one_empty_user_message(#[case] body: &str) {
    assert_eq!(
        converted(body),
        vec![value(r#"{"role":"user","content":""}"#)]
    );
}

#[test]
fn a_string_input_becomes_a_user_message() {
    assert_eq!(
        converted(r#"{"input":"hi"}"#),
        vec![value(r#"{"role":"user","content":"hi"}"#)]
    );
}

#[test]
fn instructions_become_a_leading_system_message() {
    assert_eq!(
        converted(r#"{"input":"hi","instructions":"follow"}"#),
        vec![
            value(r#"{"role":"system","content":"follow"}"#),
            value(r#"{"role":"user","content":"hi"}"#)
        ]
    );
}

#[rstest]
#[case::absent(r#"{"input":"hi"}"#, r#""hi""#)]
#[case::null(r#"{"input":null}"#, r#""""#)]
#[case::list(
    r#"{"input":[{"role":"user","content":"hi"}]}"#,
    r#"[{"role":"user","content":"hi"}]"#
)]
fn the_transformer_receives_the_input_field(#[case] body: &str, #[case] expected: &str) {
    let fake = RecordingResponses {
        messages: Vec::new(),
        seen: Mutex::new(Vec::new()),
    };
    let _messages = responses_chat_messages(&fake, &request(body));
    let recorded = {
        let seen = fake.seen.lock().expect("the responses recording lock");
        seen[0].clone()
    };
    assert_eq!(recorded.input, value(expected));
    assert_eq!(recorded.instructions, None);
}

#[test]
fn the_transformer_receives_text_instructions() {
    let fake = RecordingResponses {
        messages: Vec::new(),
        seen: Mutex::new(Vec::new()),
    };
    let _messages =
        responses_chat_messages(&fake, &request(r#"{"input":"hi","instructions":"follow"}"#));
    let recorded = {
        let seen = fake.seen.lock().expect("the responses recording lock");
        seen[0].clone()
    };
    assert_eq!(recorded.instructions.as_deref(), Some("follow"));
}

#[test]
fn a_non_text_instructions_value_is_not_forwarded() {
    let fake = RecordingResponses {
        messages: Vec::new(),
        seen: Mutex::new(Vec::new()),
    };
    let _messages = responses_chat_messages(&fake, &request(r#"{"input":"hi","instructions":5}"#));
    let recorded = {
        let seen = fake.seen.lock().expect("the responses recording lock");
        seen[0].clone()
    };
    assert_eq!(recorded.instructions, None);
}

use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_prompts::factory::custom::{CustomPromptTemplate, RoleTemplate};
use rstest::rstest;
use secrecy::SecretString;
use serde_json::{Map, Value};

use super::{
    Event, Mode, Recorder, Wrapper, base, factory, messages, now, optional, prepare, prepare_error,
    prepare_for, settings,
};
use crate::codestral_call::error::CodestralCallError;

fn template(initial: &str, final_value: &str) -> CustomPromptTemplate {
    let roles = BTreeMap::from([
        (
            "user".to_owned(),
            RoleTemplate {
                pre_message: "U:".to_owned(),
                post_message: String::new(),
            },
        ),
        (
            "assistant".to_owned(),
            RoleTemplate {
                pre_message: "A:".to_owned(),
                post_message: String::new(),
            },
        ),
    ]);
    CustomPromptTemplate {
        roles,
        initial_prompt_value: initial.to_owned(),
        final_prompt_value: final_value.to_owned(),
        bos_token: "<ignored>".to_owned(),
        eos_token: "<ignored>".to_owned(),
    }
}

#[tokio::test]
async fn a_missing_key_is_rejected() {
    let mut fixture = base();
    fixture.key = None;
    let error = prepare_error(&fixture.request()).await;
    assert!(matches!(error, CodestralCallError::MissingApiKey));
}

#[tokio::test]
async fn an_empty_key_still_builds_a_bearer_header() {
    let mut fixture = base();
    fixture.key = Some(SecretString::from(""));
    let plan = prepare_for(&fixture.request()).await.unwrap();
    assert_eq!(
        plan.headers.get("Authorization").map(String::as_str),
        Some("Bearer ")
    );
    assert_eq!(
        plan.headers.get("content-type").map(String::as_str),
        Some("application/json")
    );
}

#[tokio::test]
async fn caller_headers_replace_the_base_headers() {
    let mut fixture = base();
    fixture
        .headers
        .insert("Authorization".to_owned(), "Bearer caller".to_owned());
    fixture
        .headers
        .insert("X-Trace".to_owned(), "abc".to_owned());
    let plan = prepare_for(&fixture.request()).await.unwrap();
    assert_eq!(
        plan.headers.get("Authorization").map(String::as_str),
        Some("Bearer caller")
    );
    assert_eq!(plan.headers.get("X-Trace").map(String::as_str), Some("abc"));
}

#[tokio::test]
async fn a_case_different_caller_header_survives_the_overlay() {
    let mut fixture = base();
    fixture
        .headers
        .insert("Content-Type".to_owned(), "text/plain".to_owned());
    let plan = prepare_for(&fixture.request()).await.unwrap();
    assert_eq!(
        plan.headers.get("content-type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(
        plan.headers.get("Content-Type").map(String::as_str),
        Some("text/plain")
    );
}

#[rstest]
#[case::custom_endpoint_uses_the_base(
    Some("https://custom.example/fim"),
    Some(r#"{"custom_endpoint": true}"#),
    "https://custom.example/fim"
)]
#[case::absent_flag_uses_the_base(
    Some("https://given.example/fim"),
    None,
    "https://given.example/fim"
)]
#[case::no_base_uses_the_default(None, None, "https://codestral.mistral.ai/v1/fim/completions")]
#[case::empty_base_uses_the_default(
    Some(""),
    None,
    "https://codestral.mistral.ai/v1/fim/completions"
)]
#[case::null_flag_uses_the_base(
    Some("https://given.example/fim"),
    Some(r#"{"custom_endpoint": null}"#),
    "https://given.example/fim"
)]
#[tokio::test]
async fn the_url_follows_the_custom_endpoint_flag(
    #[case] given_base: Option<&str>,
    #[case] options: Option<&str>,
    #[case] expected: &str,
) {
    let mut fixture = base();
    fixture.api_base = given_base.map(str::to_owned);
    if let Some(text) = options {
        fixture.optional_params = optional(text);
    }
    let plan = prepare_for(&fixture.request()).await.unwrap();
    assert_eq!(plan.url, expected);
    assert!(!plan.body.contains_key("custom_endpoint"));
}

#[tokio::test]
async fn a_custom_endpoint_without_a_base_leaves_the_url_empty() {
    let mut fixture = base();
    fixture.optional_params = optional(r#"{"custom_endpoint": true}"#);
    let plan = prepare_for(&fixture.request()).await.unwrap();
    assert_eq!(plan.url, "");
}

#[tokio::test]
async fn the_custom_prompt_is_selected_by_the_exact_model_key() {
    let mut fixture = base();
    fixture.messages = messages(
        r#"[{"role": "user", "content": "write"}, {"role": "assistant", "content": "done"}]"#,
    );
    fixture
        .custom_prompts
        .insert("codestral-latest".to_owned(), template("<start>", "<end>"));
    let plan = prepare_for(&fixture.request()).await.unwrap();
    assert_eq!(plan.prompt, "<start>U:writeA:done<end>");
    assert_eq!(
        plan.body.get("prompt").and_then(Value::as_str),
        Some("<start>U:writeA:done<end>")
    );
}

#[tokio::test]
async fn another_model_spelling_uses_the_prompt_factory() {
    let mut fixture = base();
    fixture.messages = messages(r#"[{"role": "user", "content": "write"}]"#);
    fixture
        .custom_prompts
        .insert("Codestral-Latest".to_owned(), template("<lower>", "<stop>"));
    let plan = prepare_for(&fixture.request()).await.unwrap();
    assert_eq!(plan.prompt, "write");
}

#[tokio::test]
async fn config_defaults_fill_only_absent_option_keys() {
    let mut fixture = base();
    fixture.settings =
        settings(r#"{"temperature": 0.7, "max_tokens": 32, "top_p": 0.9, "stop": "\n\n"}"#);
    fixture.optional_params = optional(r#"{"temperature": null, "max_tokens": 0, "suffix": null}"#);
    let plan = prepare_for(&fixture.request()).await.unwrap();
    assert_eq!(plan.body.get("temperature"), Some(&Value::Null));
    assert_eq!(plan.body.get("max_tokens").and_then(Value::as_u64), Some(0));
    assert_eq!(plan.body.get("suffix"), Some(&Value::Null));
    assert_eq!(
        plan.body.get("top_p").and_then(Value::as_f64),
        Some(0.9_f64)
    );
    assert_eq!(plan.body.get("stop").and_then(Value::as_str), Some("\n\n"));
    assert!(!plan.body.contains_key("stream"));
}

#[rstest]
#[case::a_true_stream("true", true)]
#[case::a_false_stream("false", false)]
#[case::a_null_stream("null", false)]
#[tokio::test]
async fn the_stream_option_is_popped(#[case] given: &str, #[case] streams: bool) {
    let mut fixture = base();
    fixture.optional_params = optional(&format!(r#"{{"stream": {given}}}"#));
    let plan = prepare_for(&fixture.request()).await.unwrap();
    assert!(!plan.body.contains_key("stream"));
    assert_eq!(plan.wrapper.is_some(), streams);
}

#[tokio::test]
async fn a_sync_stream_uses_the_codestral_wrapper() {
    let mut fixture = base();
    fixture.optional_params = optional(r#"{"stream": true}"#);
    let plan = prepare_for(&fixture.request()).await.unwrap();
    assert_eq!(plan.wrapper, Some(Wrapper::Codestral));
    assert!(!plan.body.contains_key("stream"));
}

#[tokio::test]
async fn an_async_stream_reinserts_the_stream_field_after_the_pre_call() {
    let mut fixture = base();
    fixture.mode = Mode::Async;
    fixture.optional_params = optional(r#"{"stream": true}"#);
    let mut recorder = Recorder::default();
    let plan = prepare(&fixture.request(), &factory(), now(), &mut recorder)
        .await
        .unwrap();
    assert_eq!(plan.wrapper, Some(Wrapper::TextCompletionCodestral));
    assert_eq!(plan.body.get("stream"), Some(&Value::Bool(true)));
    assert_eq!(plan.timeout, None);
    let events = recorder.events();
    let Event::Pre(record) = &events[0] else {
        panic!("prepare logs pre_call first")
    };
    let logged: Map<String, Value> = serde_json::from_str(&record.body).unwrap();
    assert!(!logged.contains_key("stream"));
}

#[tokio::test]
async fn options_keep_their_precedence_over_model_and_prompt() {
    let mut fixture = base();
    fixture.messages = messages(r#"[{"role": "user", "content": "write"}]"#);
    fixture.optional_params = optional(r#"{"model": "other", "prompt": "kept"}"#);
    let plan = prepare_for(&fixture.request()).await.unwrap();
    assert_eq!(
        plan.body.get("model").and_then(Value::as_str),
        Some("other")
    );
    assert_eq!(
        plan.body.get("prompt").and_then(Value::as_str),
        Some("kept")
    );
}

#[rstest]
#[case::sync_reply(Mode::Sync, false, None)]
#[case::sync_stream(Mode::Sync, true, None)]
#[case::async_reply(Mode::Async, false, Some(Duration::from_secs(12)))]
#[case::async_stream(Mode::Async, true, None)]
#[tokio::test]
async fn the_supplied_timeout_is_used_only_by_the_async_reply(
    #[case] mode: Mode,
    #[case] streaming: bool,
    #[case] expected: Option<Duration>,
) {
    let mut fixture = base();
    fixture.mode = mode;
    if streaming {
        fixture.optional_params = optional(r#"{"stream": true}"#);
    }
    let plan = prepare_for(&fixture.request()).await.unwrap();
    assert_eq!(plan.timeout, expected);
}

#[tokio::test]
async fn the_plan_carries_every_serialized_message_in_order() {
    let mut fixture = base();
    fixture.messages = messages(
        r#"[{"role": "user", "content": "first"}, {"role": "assistant", "content": "second"}]"#,
    );
    let plan = prepare_for(&fixture.request()).await.unwrap();
    let expected: Vec<Value> = serde_json::from_str(
        r#"[{"role": "user", "content": "first"}, {"role": "assistant", "content": "second"}]"#,
    )
    .unwrap();
    assert_eq!(plan.messages, expected);
}

#[tokio::test]
async fn the_pre_call_records_the_prepared_request() {
    let mut fixture = base();
    fixture.messages = messages(r#"[{"role": "user", "content": "write"}]"#);
    fixture
        .headers
        .insert("X-Trace".to_owned(), "abc".to_owned());
    let mut recorder = Recorder::default();
    let plan = prepare(&fixture.request(), &factory(), now(), &mut recorder)
        .await
        .unwrap();
    let events = recorder.events();
    let Event::Pre(record) = &events[0] else {
        panic!("prepare logs pre_call")
    };
    assert_eq!(record.input, "write");
    assert_eq!(record.api_key.as_deref(), Some("sk-codestral"));
    assert_eq!(record.api_base, plan.url);
    assert!(!record.acompletion);
    let logged: Map<String, Value> = serde_json::from_str(&record.body).unwrap();
    assert_eq!(logged.get("prompt").and_then(Value::as_str), Some("write"));
    let headers: Map<String, Value> = serde_json::from_str(&record.headers).unwrap();
    assert_eq!(headers.get("X-Trace").and_then(Value::as_str), Some("abc"));
}

#[test]
fn the_wrapper_serializes_to_the_provider_literals() {
    assert_eq!(
        serde_json::to_value(Wrapper::Codestral).unwrap(),
        Value::from("codestral")
    );
    assert_eq!(
        serde_json::to_value(Wrapper::TextCompletionCodestral).unwrap(),
        Value::from("text-completion-codestral")
    );
}

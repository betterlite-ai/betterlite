use std::collections::BTreeMap;
use std::slice::from_ref;

use betterlite_models::decisions::Json;
use rstest::rstest;

use super::support;
use crate::message_utilities::trimming::shorten::shorten_message_to_fit_limit;
use crate::message_utilities::trimming::{
    DEFAULT_MAX_TRIM_ATTEMPTS, DEFAULT_TRIM_RATIO, TrimDefaults, TrimError, TrimRequest,
    TrimmedMessages,
    steps::{
        TokenBudget, attempt_message_addition, can_add_message, process_messages,
        process_system_message,
    },
};

const CONVERSATION: &str = r#"[
    {"role": "system", "content": "You are a helpful assistant."},
    {"role": "user", "content": "What is the capital of France?"},
    {"role": "assistant", "content": "The capital of France is Paris."},
    {"role": "user", "content": "And of Germany?"}
]"#;

const TOOL_EXCHANGE: &str = r#"[
    {"role": "user", "content": "What is the weather in Paris?"},
    {"role": "assistant", "content": null, "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "get_weather", "arguments": "{\"city\": \"Paris\"}"}}]},
    {"role": "tool", "tool_call_id": "call_1", "content": "{\"temperature\": 20}"},
    {"role": "tool", "tool_call_id": "call_2", "content": "{\"temperature\": 21}"}
]"#;

const TOOL_TAIL: &str = r#"[
    {"role": "tool", "tool_call_id": "call_1", "content": "{\"temperature\": 20}"},
    {"role": "tool", "tool_call_id": "call_2", "content": "{\"temperature\": 21}"}
]"#;

const SPENT_SYSTEM: &str = r#"[
    {"role": "system", "content": "alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon"},
    {"role": "user", "content": "u"},
    {"role": "tool", "tool_call_id": "t", "content": "x"}
]"#;

const LONG_SYSTEM: &str =
    "alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon";

const LONG_MESSAGE: &str = r#"[{"role": "user", "content": "alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon "}]"#;

const SHORTENED_40: &str = r#"[{"role": "user", "content": "alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon a.. alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon "}]"#;

const TRIMMED_40: &str = r#"[{"role": "user", "content": "alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta..a gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon "}]"#;

fn request<'request>(
    messages: &'request [Json],
    model: &'request str,
    max_tokens: Option<i64>,
) -> TrimRequest<'request> {
    TrimRequest {
        messages,
        model,
        trim_ratio: DEFAULT_TRIM_RATIO,
        return_response_tokens: false,
        max_tokens,
    }
}

#[tokio::test]
async fn trim_keeps_messages_under_the_limit_unchanged() {
    let counter = support::counter();
    let catalog = support::catalog();
    let trimmer = support::trimmer(&counter, &catalog);
    let messages = support::values(CONVERSATION);
    let outcome = trimmer
        .trim_messages(&request(&messages, "gpt-4o", Some(1000)))
        .await;
    assert_eq!(
        outcome,
        TrimmedMessages {
            messages: support::values(CONVERSATION),
            response_tokens: None,
        }
    );
}

#[tokio::test]
async fn trim_returns_the_original_messages_for_an_unknown_model() {
    let counter = support::counter();
    let catalog = support::catalog();
    let trimmer = support::trimmer(&counter, &catalog);
    let messages = support::values(CONVERSATION);
    let outcome = trimmer
        .trim_messages(&request(&messages, "model-that-does-not-exist", None))
        .await;
    assert_eq!(outcome.messages, support::values(CONVERSATION));
    assert_eq!(outcome.response_tokens, None);
}

#[tokio::test]
async fn trim_returns_the_original_messages_when_the_token_count_fails() {
    let counter = support::counter();
    let catalog = support::catalog();
    let trimmer = support::trimmer(&counter, &catalog);
    let messages = support::values(
        r#"[{"role": "user", "content": [{"type": "image_url", "image_url": {"url": "https://example.com/x.png"}}]}]"#,
    );
    let outcome = trimmer
        .trim_messages(&request(&messages, "gpt-4o", Some(5)))
        .await;
    assert_eq!(outcome.messages, messages);
    assert_eq!(outcome.response_tokens, None);
}

#[rstest]
#[case::below_a_system_message_budget(
    r#"[{"role": "system", "content": "hi"}]"#,
    7,
    r#"[{"role": "system", "content": "hi"}]"#
)]
#[case::a_spent_system_budget(
    SPENT_SYSTEM,
    22,
    r#"[{"role": "system", "content": "alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon"}]"#
)]
#[case::a_system_budget_with_room_for_the_tool_tail(
    SPENT_SYSTEM,
    23,
    r#"[{"role": "system", "content": "alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon"}, {"role": "tool", "tool_call_id": "t", "content": "x"}]"#
)]
#[case::a_dropped_message_tail(CONVERSATION, 30, r#"[{"role": "system", "content": "You are a helpful assistant."}, {"role": "user", "content": "And of Germany?"}]"#)]
#[case::one_message_dropped(CONVERSATION, 35, r#"[{"role": "system", "content": "You are a helpful assistant."}, {"role": "assistant", "content": "The capital of France is Paris."}, {"role": "user", "content": "And of Germany?"}]"#)]
#[tokio::test]
async fn trim_matches_litellm_for_a_plain_conversation(
    #[case] input: &str,
    #[case] max_tokens: i64,
    #[case] expected: &str,
) {
    let counter = support::counter();
    let catalog = support::catalog();
    let trimmer = support::trimmer(&counter, &catalog);
    let messages = support::values(input);
    let outcome = trimmer
        .trim_messages(&request(&messages, "gpt-4o", Some(max_tokens)))
        .await;
    assert_eq!(outcome.messages, support::values(expected));
}

#[rstest]
#[case::shortened(16, "alpha beta gamma delta ..eta gamma delta epsilon", 0)]
#[case::unchanged(
    30,
    "alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon",
    8
)]
#[tokio::test]
async fn process_system_message_shortens_a_long_system_prompt(
    #[case] max_tokens: i64,
    #[case] content: &str,
    #[case] remaining: i64,
) {
    let counter = support::counter();
    let catalog = support::catalog();
    let trimmer = support::trimmer(&counter, &catalog);
    let (event, left) = process_system_message(&trimmer, LONG_SYSTEM, max_tokens, "gpt-4o")
        .await
        .unwrap();
    assert_eq!(
        event,
        support::value(&format!(r#"{{"role": "system", "content": "{content}"}}"#))
    );
    assert_eq!(left, remaining);
}

#[tokio::test]
async fn trim_keeps_the_tool_tail_and_drops_older_messages() {
    let counter = support::counter();
    let catalog = support::catalog();
    let trimmer = support::trimmer(&counter, &catalog);
    let messages = support::values(TOOL_EXCHANGE);
    let outcome = trimmer
        .trim_messages(&request(&messages, "gpt-4o", Some(10)))
        .await;
    assert_eq!(outcome.messages, support::values(TOOL_TAIL));
}

#[tokio::test]
async fn trim_keeps_the_tool_tail_and_the_calling_assistant_message() {
    let counter = support::counter();
    let catalog = support::catalog();
    let trimmer = support::trimmer(&counter, &catalog);
    let messages = support::values(TOOL_EXCHANGE);
    let outcome = trimmer
        .trim_messages(&request(&messages, "gpt-4o", Some(20)))
        .await;
    assert_eq!(
        outcome.messages,
        support::values(
            r#"[
    {"role": "assistant", "content": null, "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "get_weather", "arguments": "{\"city\": \"Paris\"}"}}]},
    {"role": "tool", "tool_call_id": "call_1", "content": "{\"temperature\": 20}"},
    {"role": "tool", "tool_call_id": "call_2", "content": "{\"temperature\": 21}"}
]"#
        )
    );
}

#[tokio::test]
async fn trim_joins_system_messages_with_a_newline() {
    let counter = support::counter();
    let catalog = support::catalog();
    let trimmer = support::trimmer(&counter, &catalog);
    let messages = support::values(
        r#"[
    {"role": "system", "content": "sys1"},
    {"role": "user", "content": "u1"},
    {"role": "system", "content": "sys2"},
    {"role": "tool", "tool_call_id": "t", "content": "tool"}
]"#,
    );
    let outcome = trimmer
        .trim_messages(&request(&messages, "gpt-4o", Some(13)))
        .await;
    assert_eq!(
        outcome.messages,
        support::values(
            r#"[
    {"role": "system", "content": "sys1\nsys2"},
    {"role": "tool", "tool_call_id": "t", "content": "tool"}
]"#
        )
    );
}

#[tokio::test]
async fn trim_shortens_a_message_to_fit_the_limit() {
    let counter = support::counter();
    let catalog = support::catalog();
    let trimmer = support::trimmer(&counter, &catalog);
    let messages = support::values(LONG_MESSAGE);
    let outcome = trimmer
        .trim_messages(&request(&messages, "gpt-4o", Some(40)))
        .await;
    assert_eq!(outcome.messages, support::values(TRIMMED_40));
}

#[tokio::test]
async fn trim_reports_the_response_tokens_after_trimming() {
    let counter = support::counter();
    let catalog = support::catalog();
    let trimmer = support::trimmer(&counter, &catalog);
    let messages = support::values(CONVERSATION);
    let mut options = request(&messages, "gpt-4o", Some(43));
    options.return_response_tokens = true;
    let outcome = trimmer.trim_messages(&options).await;
    assert_eq!(
        outcome.messages,
        support::values(
            r#"[
    {"role": "system", "content": "You are a helpful assistant."},
    {"role": "assistant", "content": "The capital of France is Paris."},
    {"role": "user", "content": "And of Germany?"}
]"#
        )
    );
    assert_eq!(outcome.response_tokens, Some(-2));
}

#[tokio::test]
async fn trim_returns_no_response_tokens_on_the_early_return() {
    let counter = support::counter();
    let catalog = support::catalog();
    let trimmer = support::trimmer(&counter, &catalog);
    let messages = support::values(CONVERSATION);
    let mut options = request(&messages, "gpt-4o", Some(44));
    options.return_response_tokens = true;
    let outcome = trimmer.trim_messages(&options).await;
    assert_eq!(outcome.messages, support::values(CONVERSATION));
    assert_eq!(outcome.response_tokens, None);
}

#[tokio::test]
async fn trim_keeps_the_caller_messages_untouched() {
    let counter = support::counter();
    let catalog = support::catalog();
    let trimmer = support::trimmer(&counter, &catalog);
    let messages = support::values(LONG_MESSAGE);
    let _trimmed = trimmer
        .trim_messages(&request(&messages, "gpt-4o", Some(15)))
        .await;
    assert_eq!(messages, support::values(LONG_MESSAGE));
}

#[rstest]
#[case::max_input_tokens_scaled_by_the_ratio(
    "gpt-4o",
    0.000_3_f64,
    r#"[{"role": "system", "content": "You are a helpful assistant."}, {"role": "assistant", "content": "The capital of France is Paris."}, {"role": "user", "content": "And of Germany?"}]"#
)]
#[case::max_input_tokens_that_leave_room("gpt-4o", DEFAULT_TRIM_RATIO, CONVERSATION)]
#[case::max_tokens_without_max_input_tokens(
    "together-ai-8.1b-21b",
    0.03_f64,
    r#"[{"role": "system", "content": "You are a helpful assistant."}, {"role": "assistant", "content": "...."}, {"role": "user", "content": "And of Germany?"}]"#
)]
#[case::max_input_tokens_without_max_tokens_raises_in_litellm(
    "minimax/MiniMax-M2",
    0.000_1_f64,
    CONVERSATION
)]
#[tokio::test]
async fn trim_derives_the_limit_from_the_catalog_and_the_ratio(
    #[case] model: &str,
    #[case] trim_ratio: f64,
    #[case] expected: &str,
) {
    let counter = support::counter();
    let catalog = support::catalog();
    let trimmer = support::trimmer(&counter, &catalog);
    let messages = support::values(CONVERSATION);
    let mut options = request(&messages, model, None);
    options.trim_ratio = trim_ratio;
    let outcome = trimmer.trim_messages(&options).await;
    assert_eq!(outcome.messages, support::values(expected));
}

#[tokio::test]
async fn token_count_matches_litellm() {
    let counter = support::counter();
    let catalog = support::catalog();
    let trimmer = support::trimmer(&counter, &catalog);
    let messages = support::values(CONVERSATION);
    assert_eq!(
        trimmer.get_token_count(&messages, "gpt-4o").await.unwrap(),
        43
    );
    assert_eq!(
        trimmer
            .get_token_count(&messages, "gpt-3.5-turbo")
            .await
            .unwrap(),
        43
    );
}

#[tokio::test]
async fn process_messages_builds_the_final_list_from_the_newest_message() {
    let counter = support::counter();
    let catalog = support::catalog();
    let trimmer = support::trimmer(&counter, &catalog);
    let messages = support::values(TOOL_EXCHANGE);
    let processed = process_messages(&trimmer, &messages, 20, "gpt-4o")
        .await
        .unwrap();
    assert_eq!(
        processed,
        support::values(
            r#"[{"role": "tool", "tool_call_id": "call_2", "content": "{\"temperature\": 21}"}]"#
        )
    );
}

#[tokio::test]
async fn attempt_message_addition_shortens_a_message_that_does_not_fit() {
    let counter = support::counter();
    let catalog = support::catalog();
    let trimmer = support::trimmer(&counter, &catalog);
    let message = support::value(
        r#"{"role": "user", "content": "alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon "}"#,
    );
    let budget = TokenBudget {
        available: 40,
        max: 40,
    };
    let added = attempt_message_addition(&trimmer, &[], &message, budget, "gpt-4o")
        .await
        .unwrap();
    assert_eq!(added, support::values(SHORTENED_40));
}

#[tokio::test]
async fn attempt_message_addition_drops_a_function_call_message_that_does_not_fit() {
    let counter = support::counter();
    let catalog = support::catalog();
    let trimmer = support::trimmer(&counter, &catalog);
    let message = support::value(
        r#"{"role": "assistant", "content": null, "function_call": {"name": "f", "arguments": "{}"}}"#,
    );
    let budget = TokenBudget {
        available: 3,
        max: 3,
    };
    let added = attempt_message_addition(&trimmer, &[], &message, budget, "gpt-4o")
        .await
        .unwrap();
    assert!(added.is_empty());
}

#[tokio::test]
async fn can_add_message_accepts_an_exact_fit() {
    let counter = support::counter();
    let catalog = support::catalog();
    let trimmer = support::trimmer(&counter, &catalog);
    let system = support::value(r#"{"role": "system", "content": "You are a helpful assistant."}"#);
    let message = support::value(r#"{"role": "user", "content": "hi"}"#);
    assert!(
        can_add_message(&trimmer, &message, from_ref(&system), 18, "gpt-4o")
            .await
            .unwrap()
    );
    assert!(
        !can_add_message(&trimmer, &message, from_ref(&system), 17, "gpt-4o")
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn shorten_reports_the_attempt_limit_error() {
    let counter = support::counter();
    let catalog = support::catalog();
    let mut trimmer = support::trimmer(&counter, &catalog);
    trimmer.max_attempts = 1;
    let message = support::value(
        r#"{"role": "user", "content": "alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon "}"#,
    );
    let outcome =
        shorten_message_to_fit_limit(&trimmer, &message, 2, "claude-3-5-sonnet-20241022", true)
            .await;
    assert!(matches!(
        outcome,
        Err(TrimError::NotShortened { attempts: 1 })
    ));
}

#[tokio::test]
async fn shorten_leaves_gpt_messages_under_eleven_tokens_alone() {
    let counter = support::counter();
    let catalog = support::catalog();
    let trimmer = support::trimmer(&counter, &catalog);
    let message = support::value(r#"{"role": "user", "content": "hello world"}"#);
    let outcome = shorten_message_to_fit_limit(&trimmer, &message, 5, "gpt-4o", false)
        .await
        .unwrap();
    assert_eq!(outcome, message);
}

#[tokio::test]
async fn shorten_removes_characters_from_the_middle() {
    let counter = support::counter();
    let catalog = support::catalog();
    let trimmer = support::trimmer(&counter, &catalog);
    let message = support::values(LONG_MESSAGE).remove(0);
    let outcome = shorten_message_to_fit_limit(&trimmer, &message, 15, "gpt-4o", false)
        .await
        .unwrap();
    assert_eq!(
        outcome,
        support::value(r#"{"role": "user", "content": "alpha beta gamma ..ma delta epsilon "}"#)
    );
}

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[rstest]
#[case::unset(&[], DEFAULT_TRIM_RATIO, DEFAULT_MAX_TRIM_ATTEMPTS)]
#[case::empty(&[("DEFAULT_TRIM_RATIO", ""), ("MAX_TOKEN_TRIMMING_ATTEMPTS", " ")], DEFAULT_TRIM_RATIO, DEFAULT_MAX_TRIM_ATTEMPTS)]
#[case::ratio_only(&[("DEFAULT_TRIM_RATIO", "0.5")], 0.5_f64, DEFAULT_MAX_TRIM_ATTEMPTS)]
#[case::attempts_only(&[("MAX_TOKEN_TRIMMING_ATTEMPTS", "3")], DEFAULT_TRIM_RATIO, 3_usize)]
#[case::both(&[("DEFAULT_TRIM_RATIO", " 0.25 "), ("MAX_TOKEN_TRIMMING_ATTEMPTS", "0")], 0.25_f64, 0_usize)]
fn trim_defaults_read_the_trimming_variables(
    #[case] pairs: &[(&str, &str)],
    #[case] trim_ratio: f64,
    #[case] max_attempts: usize,
) {
    let loaded = TrimDefaults::load(&environment(pairs)).unwrap();
    assert_eq!(
        loaded,
        TrimDefaults {
            trim_ratio,
            max_attempts
        }
    );
}

#[rstest]
#[case::ratio("DEFAULT_TRIM_RATIO", "three quarters")]
#[case::attempts("MAX_TOKEN_TRIMMING_ATTEMPTS", "10.5")]
fn trim_defaults_name_the_variable_they_cannot_read(#[case] name: &str, #[case] value: &str) {
    let error = TrimDefaults::load(&environment(&[(name, value)])).unwrap_err();
    let TrimError::Environment(source) = error else {
        panic!("expected an environment error");
    };
    assert!(source.to_string().contains(name));
}

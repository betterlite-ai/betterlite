use std::borrow::Cow;

use betterlite_tokenizer::count::error::CountError;
use rust_decimal::Decimal;

use super::support::{
    Call, FixtureCounter, TOKEN_COUNTER_FAILED, UsageCase, chunks, fixture_choices,
    fixture_completion_details, fixture_details, fixture_hooks, fixture_messages, fixture_usage,
    real_counter, simple_case, simple_usage,
};
use crate::stream_usage_assembly::details::{
    apply_grounding_request_counts, attach_cache_breakdown, capture_cache_breakdown,
};
use crate::stream_usage_assembly::wire::{CacheCreationTokenDetails, Usage};
use crate::stream_usage_assembly::{
    UsageAssemblyError, UsageRequest, calculate_usage, count_reasoning_tokens,
};

pub(super) async fn token_fallback_and_failures() {
    prompt_fallbacks().await;
    prompt_failures_are_swallowed().await;
    completion_estimation_and_failures().await;
    reported_zero_is_never_estimated().await;
}

async fn prompt_fallbacks() {
    let counter = FixtureCounter::counting(7, 2, 4);
    let parsed = chunks(r#"[{"choices":[]}]"#);
    let messages = fixture_messages();
    let case = UsageCase {
        counter: &counter,
        chunks: &parsed,
        messages: Some(&messages),
        reasoning_tokens: None,
        count_prompt_tokens: None,
    };
    let (result, verbose) = case.run().await;
    assert_eq!(
        result
            .expect("the messages counter fills the prompt")
            .prompt_tokens,
        4
    );
    assert!(verbose.is_empty());
    prompt_callback_wins().await;
}

async fn prompt_callback_wins() {
    let counter = FixtureCounter::counting(7, 2, 4);
    let parsed = chunks(r#"[{"choices":[]}]"#);
    let messages = fixture_messages();
    let prompt = || Ok(11);
    let case = UsageCase {
        counter: &counter,
        chunks: &parsed,
        messages: Some(&messages),
        reasoning_tokens: None,
        count_prompt_tokens: Some(&prompt),
    };
    let (result, _) = case.run().await;
    let usage = result.expect("the supplied callback wins");
    assert_eq!(usage.prompt_tokens, 11);
    assert_eq!(usage.total_tokens, 18);
    assert_eq!(
        counter.calls(),
        vec![Call::ResponseText {
            model: "gpt-4o".to_owned(),
            text: "hi".to_owned()
        }]
    );
}

async fn prompt_failures_are_swallowed() {
    failed_message_count_is_swallowed().await;
    failed_prompt_callback_is_swallowed().await;
    missing_messages_fail_the_prompt_count().await;
}

async fn failed_message_count_is_swallowed() {
    let parsed = chunks(r#"[{"choices":[]}]"#);
    let messages = fixture_messages();
    let counter = FixtureCounter::failing_messages();
    let case = UsageCase {
        counter: &counter,
        chunks: &parsed,
        messages: Some(&messages),
        reasoning_tokens: None,
        count_prompt_tokens: None,
    };
    let (result, verbose) = case.run().await;
    assert_eq!(
        result
            .expect("a failed prompt count is swallowed")
            .prompt_tokens,
        0
    );
    assert_eq!(verbose, vec![TOKEN_COUNTER_FAILED.to_owned()]);
}

async fn failed_prompt_callback_is_swallowed() {
    let parsed = chunks(r#"[{"choices":[]}]"#);
    let messages = fixture_messages();
    let counter = FixtureCounter::counting(7, 2, 4);
    let raising = || Err(CountError::MissingInput);
    let case = UsageCase {
        counter: &counter,
        chunks: &parsed,
        messages: Some(&messages),
        reasoning_tokens: None,
        count_prompt_tokens: Some(&raising),
    };
    let (result, verbose) = case.run().await;
    assert_eq!(
        result
            .expect("a failed callback is swallowed")
            .prompt_tokens,
        0
    );
    assert_eq!(verbose, vec![TOKEN_COUNTER_FAILED.to_owned()]);
}

async fn missing_messages_fail_the_prompt_count() {
    let parsed = chunks(r#"[{"choices":[]}]"#);
    let counter = FixtureCounter::counting(7, 2, 4);
    let case = UsageCase {
        counter: &counter,
        chunks: &parsed,
        messages: None,
        reasoning_tokens: None,
        count_prompt_tokens: None,
    };
    let (result, verbose) = case.run().await;
    assert_eq!(
        result
            .expect("missing messages fail the prompt count")
            .prompt_tokens,
        0
    );
    assert_eq!(verbose, vec![TOKEN_COUNTER_FAILED.to_owned()]);
    assert_eq!(
        counter.calls(),
        vec![
            Call::Messages {
                model: "gpt-4o".to_owned(),
                messages: None
            },
            Call::ResponseText {
                model: "gpt-4o".to_owned(),
                text: "hi".to_owned()
            },
        ]
    );
}

async fn completion_estimation_and_failures() {
    reasoning_extends_the_estimate().await;
    failed_completion_count_propagates().await;
}

async fn reasoning_extends_the_estimate() {
    let parsed = chunks(r#"[{"choices":[]}]"#);
    let messages = fixture_messages();
    let counter = FixtureCounter::counting(7, 2, 4);
    let case = UsageCase {
        counter: &counter,
        chunks: &parsed,
        messages: Some(&messages),
        reasoning_tokens: Some(3),
        count_prompt_tokens: None,
    };
    let (result, _) = case.run().await;
    let usage = result.expect("reasoning tokens extend the estimate");
    assert_eq!(usage.completion_tokens, 10);
    assert_eq!(
        usage.completion_tokens_details,
        Some(fixture_completion_details(
            r#"{"reasoning_tokens":3,"text_tokens":7}"#
        ))
    );
}

async fn failed_completion_count_propagates() {
    let parsed = chunks(r#"[{"choices":[]}]"#);
    let messages = fixture_messages();
    let counter = FixtureCounter::failing_text();
    let case = UsageCase {
        counter: &counter,
        chunks: &parsed,
        messages: Some(&messages),
        reasoning_tokens: None,
        count_prompt_tokens: None,
    };
    let (result, _) = case.run().await;
    assert!(matches!(
        result,
        Err(UsageAssemblyError::CompletionCount(..))
    ));
}

async fn reported_zero_is_never_estimated() {
    let counter = FixtureCounter::counting(7, 2, 4);
    let reported = simple_usage(
        &counter,
        r#"[{"usage":{"prompt_tokens":0,"completion_tokens":0}}]"#,
    )
    .await;
    assert_eq!(
        reported,
        fixture_usage(r#"{"prompt_tokens":0,"completion_tokens":0,"total_tokens":0}"#)
    );

    let partial = simple_usage(&counter, r#"[{"usage":{"prompt_tokens":2}}]"#).await;
    assert_eq!(
        partial,
        fixture_usage(r#"{"prompt_tokens":2,"completion_tokens":0,"total_tokens":2}"#)
    );
    assert!(counter.calls().is_empty());
}

pub(super) async fn reasoning_usage() {
    reasoning_content_is_counted().await;
    supplied_reasoning_creates_details().await;
    reasoning_is_capped().await;
}

async fn reasoning_content_is_counted() {
    let choices = fixture_choices(
        r#"[{"message":{"reasoning_content":"abc"}},{"message":{"reasoning_content":""}},{"message":{"reasoning_content":"def"}}]"#,
    );
    let counter = FixtureCounter::counting(0, 3, 0);
    let sum = count_reasoning_tokens(&counter, &choices)
        .await
        .expect("reasoning counts succeed");
    assert_eq!(sum, Some(6));
    assert_eq!(
        counter.calls(),
        vec![
            Call::Text {
                text: "abc".to_owned()
            },
            Call::Text {
                text: "def".to_owned()
            },
        ]
    );

    let empty = fixture_choices(r#"[{"message":{"content":"plain"}}]"#);
    assert_eq!(
        count_reasoning_tokens(&counter, &empty)
            .await
            .expect("no reasoning is no count"),
        None
    );

    let single = fixture_choices(r#"[{"message":{"reasoning_content":"abc"}}]"#);
    let failing = FixtureCounter::failing_text();
    assert!(matches!(
        count_reasoning_tokens(&failing, &single).await,
        Err(UsageAssemblyError::CompletionCount(..))
    ));
}

async fn supplied_reasoning_creates_details() {
    let counter = FixtureCounter::counting(7, 2, 4);
    let plain = r#"[{"usage":{"prompt_tokens":1,"completion_tokens":10}}]"#;
    assert_reasoning_details(
        &counter,
        plain,
        4,
        r#"{"reasoning_tokens":4,"text_tokens":6}"#,
    )
    .await;
    assert_reasoning_details(
        &counter,
        plain,
        0,
        r#"{"reasoning_tokens":0,"text_tokens":10}"#,
    )
    .await;
    assert_reasoning_details(
        &counter,
        r#"[{"usage":{"prompt_tokens":1,"completion_tokens":10,"completion_tokens_details":{"reasoning_tokens":2,"text_tokens":3}}}]"#,
        4,
        r#"{"reasoning_tokens":2,"text_tokens":3}"#,
    )
    .await;
    assert_reasoning_details(
        &counter,
        r#"[{"usage":{"prompt_tokens":1,"completion_tokens":10,"completion_tokens_details":{"text_tokens":3}}}]"#,
        4,
        r#"{"reasoning_tokens":4,"text_tokens":3}"#,
    )
    .await;
}

async fn assert_reasoning_details(
    counter: &FixtureCounter,
    source: &str,
    reasoning: i64,
    expected: &str,
) {
    let usage = reasoning_case(counter, Some(reasoning), source).await;
    assert_eq!(
        usage.completion_tokens_details,
        Some(fixture_completion_details(expected))
    );
}

async fn reasoning_case(counter: &FixtureCounter, reasoning: Option<i64>, source: &str) -> Usage {
    let parsed = chunks(source);
    let case = UsageCase {
        counter,
        chunks: &parsed,
        messages: None,
        reasoning_tokens: reasoning,
        count_prompt_tokens: None,
    };
    case.run()
        .await
        .0
        .expect("the reasoning usage is calculated")
}

async fn reasoning_is_capped() {
    let counter = FixtureCounter::counting(7, 2, 4);
    let capped = reasoning_case(
        &counter,
        Some(5),
        r#"[{"usage":{"prompt_tokens":1,"completion_tokens":2}}]"#,
    )
    .await;
    assert_eq!(
        capped.completion_tokens_details,
        Some(fixture_completion_details(
            r#"{"reasoning_tokens":2,"text_tokens":0}"#
        ))
    );

    let negative = reasoning_case(
        &counter,
        Some(5),
        r#"[{"usage":{"prompt_tokens":1,"completion_tokens":-2}}]"#,
    )
    .await;
    assert_eq!(
        negative.completion_tokens_details,
        Some(fixture_completion_details(
            r#"{"reasoning_tokens":-2,"text_tokens":0}"#
        ))
    );

    let below_zero = reasoning_case(
        &counter,
        Some(-3),
        r#"[{"usage":{"prompt_tokens":1,"completion_tokens":10}}]"#,
    )
    .await;
    assert_eq!(
        below_zero.completion_tokens_details,
        Some(fixture_completion_details(
            r#"{"reasoning_tokens":0,"text_tokens":10}"#
        ))
    );
}

pub(super) async fn usage_detail_copying() {
    attachment_copying();
    grounding_copying();
    capture_keeps_incoming_or_current();
    final_usage_keeps_every_field().await;
}

fn attachment_copying() {
    let details = fixture_details(r#"{"cached_tokens":2}"#);
    let cache = CacheCreationTokenDetails {
        ephemeral_5m_input_tokens: None,
        ephemeral_1h_input_tokens: Some(5),
    };
    let updated =
        attach_cache_breakdown(Some(&details), Some(&cache)).expect("details stay present");
    assert!(matches!(updated, Cow::Owned(_)));
    assert_eq!(
        updated.into_owned(),
        fixture_details(
            r#"{"cached_tokens":2,"cache_creation_token_details":{"ephemeral_1h_input_tokens":5}}"#
        )
    );

    let kept = attach_cache_breakdown(Some(&details), None).expect("details stay");
    assert!(matches!(kept, Cow::Borrowed(_)));
    assert!(attach_cache_breakdown(None, Some(&cache)).is_none());

    let existing = fixture_details(
        r#"{"cached_tokens":2,"cache_creation_token_details":{"ephemeral_5m_input_tokens":1}}"#,
    );
    let untouched = attach_cache_breakdown(Some(&existing), Some(&cache)).expect("details stay");
    assert!(matches!(untouched, Cow::Borrowed(_)));
}

fn grounding_copying() {
    let details = fixture_details(r#"{"cached_tokens":2}"#);
    let borrowed =
        apply_grounding_request_counts(Some(&details), None, None).expect("details stay");
    assert!(matches!(borrowed, Cow::Borrowed(_)));
    let updated = apply_grounding_request_counts(Some(&details), Some(0), None)
        .expect("a zero request count still updates");
    assert!(matches!(updated, Cow::Owned(_)));
    assert_eq!(
        updated.into_owned(),
        fixture_details(r#"{"cached_tokens":2,"web_search_requests":0}"#)
    );
    assert!(apply_grounding_request_counts(None, None, None).is_none());
}

fn capture_keeps_incoming_or_current() {
    let details = fixture_details(
        r#"{"cached_tokens":2,"cache_creation_token_details":{"ephemeral_5m_input_tokens":1}}"#,
    );
    let current = CacheCreationTokenDetails {
        ephemeral_5m_input_tokens: None,
        ephemeral_1h_input_tokens: Some(5),
    };
    assert_eq!(
        capture_cache_breakdown(Some(&details), Some(current.clone())),
        Some(CacheCreationTokenDetails {
            ephemeral_5m_input_tokens: Some(1),
            ephemeral_1h_input_tokens: None,
        })
    );
    let plain = fixture_details(r#"{"cached_tokens":2}"#);
    assert_eq!(
        capture_cache_breakdown(Some(&plain), Some(current.clone())),
        Some(current)
    );
    assert_eq!(capture_cache_breakdown(None, None), None);
}

async fn final_usage_keeps_every_field() {
    let counter = FixtureCounter::counting(7, 2, 4);
    let source = chunks(
        r#"[{"usage":{"prompt_tokens":4,"cache_creation_input_tokens":3,"prompt_tokens_details":{"cached_tokens":1,"cache_creation_token_details":{"ephemeral_1h_input_tokens":5}},"server_tool_use":{"web_search_requests":2},"cost":0.25}},{"usage":{"prompt_tokens":6,"cache_read_input_tokens":7}}]"#,
    );
    let original = source.clone();
    let (result, _) = simple_case(&counter, &source).run().await;
    let usage = result.expect("the final usage keeps every field");
    assert_eq!(usage.cache_creation_input_tokens, Some(3));
    assert_eq!(usage.cache_read_input_tokens, Some(7));
    assert_eq!(
        usage.prompt_tokens_details,
        Some(fixture_details(
            r#"{"cached_tokens":7,"cache_write_tokens":3,"cache_creation_tokens":3,"cache_creation_token_details":{"ephemeral_1h_input_tokens":5}}"#
        ))
    );
    assert_eq!(
        usage
            .server_tool_use
            .as_ref()
            .and_then(|tool_use| tool_use.web_search_requests),
        Some(2)
    );
    assert_eq!(usage.cost, Some(Decimal::new(25, 2)));
    assert_eq!(source, original);
}

#[tokio::test]
async fn real_counter_estimates_unreported_counts() {
    let counter = real_counter();
    let parsed = chunks(r#"[{"choices":[]}]"#);
    let messages = fixture_messages();
    let hooks = fixture_hooks();
    let text = "\u{41f}\u{440}\u{438}\u{432}\u{435}\u{442}, \u{43a}\u{430}\u{43a} \u{434}\u{435}\u{43b}\u{430}? The quick brown fox jumps over 12345 lazy dogs!";
    let request = UsageRequest {
        chunks: &parsed,
        model: "gpt-4o",
        completion_output: text,
        messages: Some(&messages),
        reasoning_tokens: None,
        count_prompt_tokens: None,
    };
    let usage = calculate_usage(&counter, &hooks, &request)
        .await
        .expect("the real counter estimates");
    // The messages estimate keeps the reply-priming tokens LiteLLM adds (5 without them).
    assert_eq!(usage.prompt_tokens, 8);
    // The same text is 20 through the no-model counter, so 18 pins the gpt-4o tokenizer.
    assert_eq!(usage.completion_tokens, 18);
    let reasoning = format!(r#"[{{"message":{{"reasoning_content":"{text}"}}}}]"#);
    let choices = fixture_choices(&reasoning);
    assert_eq!(
        count_reasoning_tokens(&counter, &choices)
            .await
            .expect("the real counter counts reasoning"),
        Some(20)
    );
}

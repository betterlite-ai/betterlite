#![cfg(test)]

use std::borrow::Cow;

use betterlite_tokenizer::count::error::CountError;
use rstest::rstest;
use rust_decimal::Decimal;

use super::details::{
    apply_grounding_request_counts, attach_cache_breakdown, capture_cache_breakdown,
};
use super::wire::{CacheCreationTokenDetails, Usage};
use super::{UsageAssemblyError, count_reasoning_tokens};

mod support;

use support::{
    Call, FixtureCounter, TOKEN_COUNTER_FAILED, UsageCase, chunks, fixture_choices,
    fixture_completion_details, fixture_details, fixture_messages, fixture_usage, simple_case,
    simple_usage,
};

#[derive(Clone, Copy, Debug)]
enum Proof {
    UsageSourceAndCounts,
    CacheAndDetails,
    GroundingAndPricing,
    AnthropicCursorReset,
    TokenFallbackAndFailures,
    ReasoningUsage,
    UsageDetailCopying,
}

#[rstest]
#[case::usage_source_and_counts(Proof::UsageSourceAndCounts)]
#[case::cache_and_details(Proof::CacheAndDetails)]
#[case::grounding_and_pricing(Proof::GroundingAndPricing)]
#[case::anthropic_cursor_reset(Proof::AnthropicCursorReset)]
#[case::token_fallback_and_failures(Proof::TokenFallbackAndFailures)]
#[case::reasoning_usage(Proof::ReasoningUsage)]
#[case::usage_detail_copying(Proof::UsageDetailCopying)]
#[tokio::test]
async fn stream_usage_assembly_cases(#[case] proof: Proof) {
    match proof {
        Proof::UsageSourceAndCounts => usage_source_and_counts().await,
        Proof::CacheAndDetails => cache_and_details().await,
        Proof::GroundingAndPricing => grounding_and_pricing().await,
        Proof::AnthropicCursorReset => anthropic_cursor_reset().await,
        Proof::TokenFallbackAndFailures => token_fallback_and_failures().await,
        Proof::ReasoningUsage => reasoning_usage().await,
        Proof::UsageDetailCopying => usage_detail_copying().await,
    }
}

async fn usage_source_and_counts() {
    counts_replace_and_report_zero().await;
    absent_usage_is_estimated().await;
    usage_sources_are_preferred().await;
    hidden_usage_needs_a_response_object().await;
    an_explicit_null_usage_stops_the_hidden_lookup().await;
}

async fn counts_replace_and_report_zero() {
    let counter = FixtureCounter::counting(7, 2, 4);
    let usage = simple_usage(
        &counter,
        r#"[{"usage":{"prompt_tokens":0,"completion_tokens":2}},{"usage":{"prompt_tokens":5,"completion_tokens":0}}]"#,
    )
    .await;
    assert_eq!(
        usage,
        fixture_usage(r#"{"prompt_tokens":5,"completion_tokens":2,"total_tokens":7}"#)
    );
    assert!(counter.calls().is_empty());

    let negative = simple_usage(
        &counter,
        r#"[{"usage":{"prompt_tokens":-2,"completion_tokens":4}},{"usage":{"prompt_tokens":0,"completion_tokens":-1}}]"#,
    )
    .await;
    assert_eq!(
        negative,
        fixture_usage(r#"{"prompt_tokens":-2,"completion_tokens":4,"total_tokens":2}"#)
    );

    let unreported = simple_usage(
        &counter,
        r#"[{"choices":[]},{"usage":{"completion_tokens":-1}}]"#,
    )
    .await;
    assert_eq!(
        unreported,
        fixture_usage(r#"{"prompt_tokens":0,"completion_tokens":-1,"total_tokens":-1}"#)
    );

    let empty_record = simple_usage(&counter, r#"[{"usage":{}}]"#).await;
    assert_eq!(
        empty_record,
        fixture_usage(r#"{"prompt_tokens":0,"completion_tokens":0,"total_tokens":0}"#)
    );
    assert!(counter.calls().is_empty());
}

async fn absent_usage_is_estimated() {
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
        result.expect("an absent usage record is estimated"),
        fixture_usage(r#"{"prompt_tokens":4,"completion_tokens":7,"total_tokens":11}"#)
    );
    assert_eq!(
        counter.calls(),
        vec![
            Call::Messages {
                model: "gpt-4o".to_owned(),
                messages: Some(1)
            },
            Call::ResponseText {
                model: "gpt-4o".to_owned(),
                text: "hi".to_owned()
            },
        ]
    );
    assert!(verbose.is_empty());
}

async fn usage_sources_are_preferred() {
    let counter = FixtureCounter::counting(7, 2, 4);
    let preferred = simple_usage(
        &counter,
        r#"[{"form":"model_response_stream","choices":[],"usage":{"prompt_tokens":5,"completion_tokens":3},"_hidden_params":{"usage":{"prompt_tokens":9,"completion_tokens":9}}}]"#,
    )
    .await;
    assert_eq!(
        preferred,
        fixture_usage(r#"{"prompt_tokens":5,"completion_tokens":3,"total_tokens":8}"#)
    );
    assert!(counter.calls().is_empty());

    let hidden = simple_usage(
        &counter,
        r#"[{"form":"model_response","choices":[],"_hidden_params":{"usage":{"prompt_tokens":4,"completion_tokens":3}}}]"#,
    )
    .await;
    assert_eq!(
        hidden,
        fixture_usage(r#"{"prompt_tokens":4,"completion_tokens":3,"total_tokens":7}"#)
    );

    let stream_hidden = simple_usage(
        &counter,
        r#"[{"form":"model_response_stream","choices":[],"_hidden_params":{"usage":{"prompt_tokens":2,"completion_tokens":1}}}]"#,
    )
    .await;
    assert_eq!(
        stream_hidden,
        fixture_usage(r#"{"prompt_tokens":2,"completion_tokens":1,"total_tokens":3}"#)
    );
}

async fn hidden_usage_needs_a_response_object() {
    let counter = FixtureCounter::counting(7, 2, 4);
    let messages = fixture_messages();
    let dict_hidden = chunks(
        r#"[{"choices":[],"_hidden_params":{"usage":{"prompt_tokens":4,"completion_tokens":3}}}]"#,
    );
    let case = UsageCase {
        counter: &counter,
        chunks: &dict_hidden,
        messages: Some(&messages),
        reasoning_tokens: None,
        count_prompt_tokens: None,
    };
    let (result, _) = case.run().await;
    assert_eq!(
        result.expect("a dict chunk never reads hidden usage"),
        fixture_usage(r#"{"prompt_tokens":4,"completion_tokens":7,"total_tokens":11}"#)
    );
}

async fn an_explicit_null_usage_stops_the_hidden_lookup() {
    let counter = FixtureCounter::counting(7, 2, 4);
    let messages = fixture_messages();
    let explicit_null = chunks(
        r#"[{"form":"model_response_stream","choices":[],"usage":null,"_hidden_params":{"usage":{"prompt_tokens":4,"completion_tokens":3}}}]"#,
    );
    let case = UsageCase {
        counter: &counter,
        chunks: &explicit_null,
        messages: Some(&messages),
        reasoning_tokens: None,
        count_prompt_tokens: None,
    };
    let (result, _) = case.run().await;
    assert_eq!(
        result.expect("an explicit null usage attribute stops the hidden lookup"),
        fixture_usage(r#"{"prompt_tokens":4,"completion_tokens":7,"total_tokens":11}"#)
    );
}

async fn cache_and_details() {
    cache_counts_follow_prompt_side_reporting().await;
    details_are_last_wins().await;
    nested_details_survive_replacement().await;
    server_tool_use_is_coerced().await;
}

async fn cache_counts_follow_prompt_side_reporting() {
    let counter = FixtureCounter::counting(7, 2, 4);
    let creation = simple_usage(
        &counter,
        r#"[{"usage":{"cache_creation_input_tokens":7}},{"usage":{"cache_creation_input_tokens":9,"prompt_tokens":3}},{"usage":{"cache_creation_input_tokens":11}}]"#,
    )
    .await;
    assert_eq!(
        creation.prompt_tokens_details,
        Some(fixture_details(
            r#"{"cache_write_tokens":11,"cache_creation_tokens":11}"#
        ))
    );
    assert_eq!(creation.cache_creation_input_tokens, Some(11));

    let read = simple_usage(
        &counter,
        r#"[{"usage":{"cache_read_input_tokens":2}},{"usage":{"cache_read_input_tokens":5,"prompt_tokens":1}},{"usage":{"cache_read_input_tokens":8}}]"#,
    )
    .await;
    assert_eq!(
        read.prompt_tokens_details,
        Some(fixture_details(r#"{"cached_tokens":8}"#))
    );
    assert_eq!(read.cache_read_input_tokens, Some(8));
}

async fn details_are_last_wins() {
    let counter = FixtureCounter::counting(7, 2, 4);
    let completion = simple_usage(
        &counter,
        r#"[{"usage":{"completion_tokens":4,"completion_tokens_details":{"reasoning_tokens":2}}},{"usage":{"completion_tokens":5}},{"usage":{"completion_tokens":6,"completion_tokens_details":{"reasoning_tokens":3}}}]"#,
    )
    .await;
    assert_eq!(completion.completion_tokens, 6);
    assert_eq!(
        completion
            .completion_tokens_details
            .and_then(|details| details.reasoning_tokens),
        Some(3)
    );

    let mirrored = simple_usage(
        &counter,
        r#"[{"usage":{"prompt_tokens_details":{"cache_write_tokens":5}}}]"#,
    )
    .await;
    assert_eq!(
        mirrored.prompt_tokens_details,
        Some(fixture_details(
            r#"{"cache_write_tokens":5,"cache_creation_tokens":5}"#
        ))
    );
}

async fn nested_details_survive_replacement() {
    let counter = FixtureCounter::counting(7, 2, 4);
    let preserved = simple_usage(
        &counter,
        r#"[{"usage":{"prompt_tokens":3,"prompt_tokens_details":{"cached_tokens":1,"cache_creation_token_details":{"ephemeral_1h_input_tokens":5}}}},{"usage":{"prompt_tokens":4,"prompt_tokens_details":{"cached_tokens":2}}}]"#,
    )
    .await;
    assert_eq!(
        preserved.prompt_tokens_details,
        Some(fixture_details(
            r#"{"cached_tokens":2,"cache_creation_token_details":{"ephemeral_1h_input_tokens":5}}"#
        ))
    );

    let own = simple_usage(
        &counter,
        r#"[{"usage":{"prompt_tokens":3,"prompt_tokens_details":{"cache_creation_token_details":{"ephemeral_5m_input_tokens":1}}}},{"usage":{"prompt_tokens":4,"prompt_tokens_details":{"cached_tokens":2,"cache_creation_token_details":{"ephemeral_1h_input_tokens":6}}}}]"#,
    )
    .await;
    assert_eq!(
        own.prompt_tokens_details,
        Some(fixture_details(
            r#"{"cached_tokens":2,"cache_creation_token_details":{"ephemeral_1h_input_tokens":6}}"#
        ))
    );
}

async fn server_tool_use_is_coerced() {
    let counter = FixtureCounter::counting(7, 2, 4);
    let usage = simple_usage(
        &counter,
        r#"[{"usage":{"prompt_tokens":1,"server_tool_use":{"web_search_requests":2}}},{"usage":{"prompt_tokens":2,"server_tool_use":null}},{"usage":{"prompt_tokens":3,"server_tool_use":{"web_search_requests":4,"browser_open_requests":1}}}]"#,
    )
    .await;
    let tool_use = usage.server_tool_use.expect("a server tool use record");
    assert_eq!(tool_use.web_search_requests, Some(4));
    assert_eq!(tool_use.browser_open_requests, Some(1));

    let malformed = chunks(r#"[{"usage":{"prompt_tokens":1,"server_tool_use":"x"}}]"#);
    let (result, _) = simple_case(&counter, &malformed).run().await;
    assert!(matches!(
        result,
        Err(UsageAssemblyError::ServerToolUse { field }) if field == "server_tool_use"
    ));
}

async fn grounding_and_pricing() {
    grounding_counts_are_retained().await;
    cost_and_pricing_fields_are_last_wins().await;
}

async fn grounding_counts_are_retained() {
    let counter = FixtureCounter::counting(7, 2, 4);
    let usage = simple_usage(
        &counter,
        r#"[{"usage":{"prompt_tokens":1,"prompt_tokens_details":{"web_search_requests":1}}},{"usage":{"prompt_tokens":2,"prompt_tokens_details":{"google_maps_grounding_requests":0,"cached_tokens":3}}}]"#,
    )
    .await;
    assert_eq!(
        usage.prompt_tokens_details,
        Some(fixture_details(
            r#"{"cached_tokens":3,"web_search_requests":1,"google_maps_grounding_requests":0}"#
        ))
    );

    let replaced = simple_usage(
        &counter,
        r#"[{"usage":{"prompt_tokens":1,"prompt_tokens_details":{"web_search_requests":1}}},{"usage":{"prompt_tokens":2,"prompt_tokens_details":{"web_search_requests":5}}}]"#,
    )
    .await;
    assert_eq!(
        replaced
            .prompt_tokens_details
            .and_then(|details| details.web_search_requests),
        Some(5)
    );

    let created = apply_grounding_request_counts(None, Some(1), Some(2))
        .expect("grounding counts create details");
    assert_eq!(
        created.into_owned(),
        fixture_details(r#"{"web_search_requests":1,"google_maps_grounding_requests":2}"#)
    );
}

async fn cost_and_pricing_fields_are_last_wins() {
    let counter = FixtureCounter::counting(7, 2, 4);
    let replaced = simple_usage(
        &counter,
        r#"[{"usage":{"prompt_tokens":1,"cost":0.5}},{"usage":{"prompt_tokens":2,"cost":0}}]"#,
    )
    .await;
    assert_eq!(replaced.cost, Some(Decimal::ZERO));

    let kept = simple_usage(
        &counter,
        r#"[{"usage":{"prompt_tokens":1,"cost":1.25}},{"usage":{"prompt_tokens":2,"cost":null}}]"#,
    )
    .await;
    assert_eq!(kept.cost, Some(Decimal::new(125, 2)));

    let pricing = simple_usage(
        &counter,
        r#"[{"usage":{"prompt_tokens":1,"inference_geo":"us","speed":"fast"}},{"usage":{"prompt_tokens":2,"inference_geo":5,"speed":""}}]"#,
    )
    .await;
    assert_eq!(pricing.inference_geo.as_deref(), Some("us"));
    assert_eq!(pricing.speed.as_deref(), Some(""));

    let nulls = simple_usage(
        &counter,
        r#"[{"usage":{"prompt_tokens":1,"speed":"slow","inference_geo":"eu"}},{"usage":{"prompt_tokens":2,"speed":null,"inference_geo":null}}]"#,
    )
    .await;
    assert_eq!(nulls.speed.as_deref(), Some("slow"));
    assert_eq!(nulls.inference_geo.as_deref(), Some("eu"));
}

async fn anthropic_cursor_reset() {
    lone_cursor_is_estimated().await;
    completion_updates_keep_the_count().await;
    finish_reasons_are_evidence().await;
    other_providers_keep_their_counts().await;
}

async fn lone_cursor_is_estimated() {
    let counter = FixtureCounter::counting(7, 2, 4);
    let usage = simple_usage(
        &counter,
        r#"[{"_hidden_params":{"custom_llm_provider":"anthropic"},"usage":{"completion_tokens":8,"completion_tokens_details":{"reasoning_tokens":2}},"choices":[]}]"#,
    )
    .await;
    assert_eq!(usage.completion_tokens, 7);
    assert!(usage.completion_tokens_details.is_none());
    assert_eq!(
        counter.calls(),
        vec![Call::ResponseText {
            model: "gpt-4o".to_owned(),
            text: "hi".to_owned()
        }]
    );

    let zero = simple_usage(
        &counter,
        r#"[{"_hidden_params":{"custom_llm_provider":"anthropic"},"usage":{"completion_tokens":0},"choices":[]}]"#,
    )
    .await;
    assert_eq!(zero.completion_tokens, 7);
}

async fn completion_updates_keep_the_count() {
    let counter = FixtureCounter::counting(7, 2, 4);
    let two = simple_usage(
        &counter,
        r#"[{"_hidden_params":{"custom_llm_provider":"anthropic"},"usage":{"completion_tokens":8}},{"_hidden_params":{"custom_llm_provider":"anthropic"},"usage":{"completion_tokens":8}}]"#,
    )
    .await;
    assert_eq!(two.completion_tokens, 8);
    assert!(counter.calls().is_empty());

    let one = simple_usage(
        &counter,
        r#"[{"_hidden_params":{"custom_llm_provider":"anthropic"},"usage":{"completion_tokens":8}},{"_hidden_params":{"custom_llm_provider":"anthropic"},"usage":{"completion_tokens":0}}]"#,
    )
    .await;
    assert_eq!(one.completion_tokens, 7);
}

async fn finish_reasons_are_evidence() {
    let counter = FixtureCounter::counting(7, 2, 4);
    let finished = simple_usage(
        &counter,
        r#"[{"_hidden_params":{"custom_llm_provider":"anthropic"},"usage":{"completion_tokens":8},"choices":[{"finish_reason":"stop"}]}]"#,
    )
    .await;
    assert_eq!(finished.completion_tokens, 8);

    let empty = simple_usage(
        &counter,
        r#"[{"_hidden_params":{"custom_llm_provider":"anthropic"},"usage":{"completion_tokens":8},"choices":[{"finish_reason":""}]}]"#,
    )
    .await;
    assert_eq!(empty.completion_tokens, 8);

    let missing = simple_usage(
        &counter,
        r#"[{"_hidden_params":{"custom_llm_provider":"anthropic"},"usage":{"completion_tokens":8},"choices":[{"finish_reason":null}]}]"#,
    )
    .await;
    assert_eq!(missing.completion_tokens, 7);
}

async fn other_providers_keep_their_counts() {
    let counter = FixtureCounter::counting(7, 2, 4);
    let other = simple_usage(
        &counter,
        r#"[{"_hidden_params":{"custom_llm_provider":"openai"},"usage":{"completion_tokens":8},"choices":[]}]"#,
    )
    .await;
    assert_eq!(other.completion_tokens, 8);

    let no_provider = simple_usage(
        &counter,
        r#"[{"usage":{"completion_tokens":8},"choices":[]}]"#,
    )
    .await;
    assert_eq!(no_provider.completion_tokens, 8);
}

async fn token_fallback_and_failures() {
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
        vec![
            Call::Messages {
                model: "gpt-4o".to_owned(),
                messages: Some(1)
            },
            Call::ResponseText {
                model: "gpt-4o".to_owned(),
                text: "hi".to_owned()
            },
        ]
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
        Err(UsageAssemblyError::CompletionCount { .. })
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

async fn reasoning_usage() {
    reasoning_content_is_counted().await;
    supplied_reasoning_creates_details().await;
    reported_details_are_preserved().await;
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
        Err(UsageAssemblyError::CompletionCount { .. })
    ));
}

async fn supplied_reasoning_creates_details() {
    let counter = FixtureCounter::counting(7, 2, 4);
    let created = reasoning_case(
        &counter,
        Some(4),
        r#"[{"usage":{"prompt_tokens":1,"completion_tokens":10}}]"#,
    )
    .await;
    assert_eq!(
        created.completion_tokens_details,
        Some(fixture_completion_details(
            r#"{"reasoning_tokens":4,"text_tokens":6}"#
        ))
    );

    let zero = reasoning_case(
        &counter,
        Some(0),
        r#"[{"usage":{"prompt_tokens":1,"completion_tokens":10}}]"#,
    )
    .await;
    assert_eq!(
        zero.completion_tokens_details,
        Some(fixture_completion_details(
            r#"{"reasoning_tokens":0,"text_tokens":10}"#
        ))
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

async fn reported_details_are_preserved() {
    let counter = FixtureCounter::counting(7, 2, 4);
    let reported = reasoning_case(
        &counter,
        Some(4),
        r#"[{"usage":{"prompt_tokens":1,"completion_tokens":10,"completion_tokens_details":{"reasoning_tokens":2,"text_tokens":3}}}]"#,
    )
    .await;
    assert_eq!(
        reported.completion_tokens_details,
        Some(fixture_completion_details(
            r#"{"reasoning_tokens":2,"text_tokens":3}"#
        ))
    );

    let partial = reasoning_case(
        &counter,
        Some(4),
        r#"[{"usage":{"prompt_tokens":1,"completion_tokens":10,"completion_tokens_details":{"text_tokens":3}}}]"#,
    )
    .await;
    assert_eq!(
        partial.completion_tokens_details,
        Some(fixture_completion_details(
            r#"{"reasoning_tokens":4,"text_tokens":3}"#
        ))
    );
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

async fn usage_detail_copying() {
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

#![cfg(test)]

use rstest::rstest;
use rust_decimal::Decimal;

use super::UsageAssemblyError;
use super::details::apply_grounding_request_counts;
use super::wire::{CompletionTokensDetails, PromptTokensDetails};

mod estimation;
mod support;

use support::{
    Call, FixtureCounter, UsageCase, chunks, fixture_completion_details, fixture_details,
    fixture_messages, fixture_usage, simple_case, simple_usage,
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
        Proof::TokenFallbackAndFailures => estimation::token_fallback_and_failures().await,
        Proof::ReasoningUsage => estimation::reasoning_usage().await,
        Proof::UsageDetailCopying => estimation::usage_detail_copying().await,
    }
}

async fn usage_source_and_counts() {
    counts_replace_and_report_zero().await;
    absent_usage_is_estimated().await;
    usage_sources_are_preferred().await;
    hidden_usage_needs_a_response_object().await;
    usage_record_cache_mappings_are_applied().await;
    usage_record_reasoning_is_filled().await;
}

async fn usage_record_cache_mappings_are_applied() {
    let counter = FixtureCounter::counting(7, 2, 4);
    let hits = simple_usage(
        &counter,
        r#"[{"usage":{"prompt_tokens":1,"prompt_cache_hit_tokens":5}}]"#,
    )
    .await;
    assert_eq!(
        hits.prompt_tokens_details,
        Some(fixture_details(r#"{"cached_tokens":5}"#))
    );

    let creation = simple_usage(
        &counter,
        r#"[{"usage":{"prompt_tokens":1,"prompt_tokens_details":{"cache_creation_tokens":5}}}]"#,
    )
    .await;
    assert_eq!(
        creation.prompt_tokens_details,
        Some(fixture_details(
            r#"{"cache_write_tokens":5,"cache_creation_tokens":5}"#
        ))
    );

    let nested = simple_usage(
        &counter,
        r#"[{"usage":{"prompt_tokens":1,"prompt_tokens_details":{"cache_creation_input_tokens":4}}}]"#,
    )
    .await;
    assert_eq!(
        nested.prompt_tokens_details,
        Some(fixture_details(
            r#"{"cache_creation_input_tokens":4,"cache_write_tokens":4,"cache_creation_tokens":4}"#
        ))
    );
}

async fn usage_record_reasoning_is_filled() {
    let counter = FixtureCounter::counting(7, 2, 4);
    let filled = simple_usage(
        &counter,
        r#"[{"usage":{"completion_tokens":10,"reasoning_tokens":4,"completion_tokens_details":{"image_tokens":2,"audio_tokens":1}}}]"#,
    )
    .await;
    assert_eq!(
        filled.completion_tokens_details,
        Some(fixture_completion_details(
            r#"{"reasoning_tokens":4,"text_tokens":3,"image_tokens":2,"audio_tokens":1}"#
        ))
    );
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
    assert_hidden_usage_ignored(
        &counter,
        r#"[{"choices":[],"_hidden_params":{"usage":{"prompt_tokens":4,"completion_tokens":3}}}]"#,
    )
    .await;
    assert_hidden_usage_ignored(
        &counter,
        r#"[{"form":"model_response_stream","choices":[],"usage":null,"_hidden_params":{"usage":{"prompt_tokens":4,"completion_tokens":3}}}]"#,
    )
    .await;
}

async fn assert_hidden_usage_ignored(counter: &FixtureCounter, source: &str) {
    let messages = fixture_messages();
    let parsed = chunks(source);
    let case = UsageCase {
        counter,
        chunks: &parsed,
        messages: Some(&messages),
        reasoning_tokens: None,
        count_prompt_tokens: None,
    };
    let (result, _) = case.run().await;
    assert_eq!(
        result.expect("hidden usage stays unused"),
        fixture_usage(r#"{"prompt_tokens":4,"completion_tokens":7,"total_tokens":11}"#)
    );
}

async fn cache_and_details() {
    cache_counts_follow_prompt_side_reporting().await;
    cache_counts_retain_earlier_values().await;
    details_are_last_wins().await;
    empty_details_never_clear_earlier_ones().await;
    nested_details_survive_replacement().await;
    server_tool_use_is_coerced().await;
}

async fn cache_counts_retain_earlier_values() {
    let counter = FixtureCounter::counting(7, 2, 4);
    let creation = simple_usage(
        &counter,
        r#"[{"usage":{"cache_creation_input_tokens":7}},{"usage":{"cache_creation_input_tokens":0}}]"#,
    )
    .await;
    assert_eq!(creation.cache_creation_input_tokens, Some(7));
    assert_eq!(
        creation.prompt_tokens_details,
        Some(fixture_details(
            r#"{"cache_write_tokens":7,"cache_creation_tokens":7}"#
        ))
    );

    let read = simple_usage(
        &counter,
        r#"[{"usage":{"cache_read_input_tokens":2}},{"usage":{"cache_read_input_tokens":0}}]"#,
    )
    .await;
    assert_eq!(read.cache_read_input_tokens, Some(2));
    assert_eq!(
        read.prompt_tokens_details,
        Some(fixture_details(r#"{"cached_tokens":2}"#))
    );

    let replaced = simple_usage(
        &counter,
        r#"[{"usage":{"cache_creation_input_tokens":7}},{"usage":{"cache_creation_input_tokens":0,"prompt_tokens":3}}]"#,
    )
    .await;
    assert_eq!(replaced.cache_creation_input_tokens, Some(0));
    assert_eq!(
        replaced.prompt_tokens_details,
        Some(fixture_details(
            r#"{"cache_write_tokens":0,"cache_creation_tokens":0}"#
        ))
    );
}

async fn empty_details_never_clear_earlier_ones() {
    let counter = FixtureCounter::counting(7, 2, 4);
    let kept = simple_usage(
        &counter,
        r#"[{"usage":{"prompt_tokens":3,"prompt_tokens_details":{"cached_tokens":2}}},{"usage":{"prompt_tokens":4,"prompt_tokens_details":{}}}]"#,
    )
    .await;
    assert_eq!(
        kept.prompt_tokens_details,
        Some(fixture_details(r#"{"cached_tokens":2}"#))
    );

    let cleared = simple_usage(
        &counter,
        r#"[{"usage":{"prompt_tokens":3,"prompt_tokens_details":{"cached_tokens":2}}},{"usage":{"prompt_tokens":4,"prompt_tokens_details":{"cached_tokens":null}}}]"#,
    )
    .await;
    assert_eq!(
        cleared.prompt_tokens_details,
        Some(PromptTokensDetails::default())
    );
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

    let empty = simple_usage(
        &counter,
        r#"[{"usage":{"completion_tokens":5,"completion_tokens_details":{"reasoning_tokens":2}}},{"usage":{"completion_tokens":6,"completion_tokens_details":{}}}]"#,
    )
    .await;
    assert_eq!(
        empty.completion_tokens_details,
        Some(fixture_completion_details(r#"{"reasoning_tokens":2}"#))
    );

    let all_null = simple_usage(
        &counter,
        r#"[{"usage":{"completion_tokens":5,"completion_tokens_details":{"reasoning_tokens":2}}},{"usage":{"completion_tokens":6,"completion_tokens_details":{"reasoning_tokens":null}}}]"#,
    )
    .await;
    assert_eq!(
        all_null.completion_tokens_details,
        Some(CompletionTokensDetails::default())
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

    let extra_key = simple_usage(
        &counter,
        r#"[{"usage":{"prompt_tokens":1,"server_tool_use":{"web_search_requests":2,"code_execution_requests":1}}}]"#,
    )
    .await;
    assert_eq!(
        extra_key
            .server_tool_use
            .and_then(|recorded| recorded.web_search_requests),
        Some(2)
    );

    let malformed = chunks(r#"[{"usage":{"prompt_tokens":1,"server_tool_use":"x"}}]"#);
    let (result, _) = simple_case(&counter, &malformed).run().await;
    assert!(matches!(
        result,
        Err(UsageAssemblyError::ServerToolUse { field }) if field == "server_tool_use"
    ));

    let bad_count = chunks(
        r#"[{"usage":{"prompt_tokens":1,"server_tool_use":{"web_search_requests":"two"}}}]"#,
    );
    let (rejected, _) = simple_case(&counter, &bad_count).run().await;
    assert!(matches!(
        rejected,
        Err(UsageAssemblyError::ServerToolUse { field }) if field == "web_search_requests"
    ));
}

async fn grounding_and_pricing() {
    grounding_counts_are_retained().await;
    google_maps_counts_are_retained().await;
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

async fn google_maps_counts_are_retained() {
    let counter = FixtureCounter::counting(7, 2, 4);
    let maps_kept = simple_usage(
        &counter,
        r#"[{"usage":{"prompt_tokens":1,"prompt_tokens_details":{"google_maps_grounding_requests":2}}},{"usage":{"prompt_tokens":2,"prompt_tokens_details":{"cached_tokens":3}}}]"#,
    )
    .await;
    assert_eq!(
        maps_kept.prompt_tokens_details,
        Some(fixture_details(
            r#"{"cached_tokens":3,"google_maps_grounding_requests":2}"#
        ))
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
    cursor_evidence_is_positioned().await;
    other_providers_keep_their_counts().await;
}

async fn cursor_evidence_is_positioned() {
    let counter = FixtureCounter::counting(7, 2, 4);
    let later_chunk = simple_usage(
        &counter,
        r#"[{"_hidden_params":{"custom_llm_provider":"anthropic"},"usage":{"completion_tokens":8}},{"usage":{"completion_tokens":0},"choices":[{"finish_reason":"stop"}]}]"#,
    )
    .await;
    assert_eq!(later_chunk.completion_tokens, 8);

    let second_choice = simple_usage(
        &counter,
        r#"[{"_hidden_params":{"custom_llm_provider":"anthropic"},"usage":{"completion_tokens":8},"choices":[{},{"finish_reason":"stop"}]}]"#,
    )
    .await;
    assert_eq!(second_choice.completion_tokens, 8);

    let provider_on_first = simple_usage(
        &counter,
        r#"[{"_hidden_params":{"custom_llm_provider":"anthropic"},"usage":{"completion_tokens":8}},{"usage":{"completion_tokens":0}}]"#,
    )
    .await;
    assert_eq!(provider_on_first.completion_tokens, 7);

    let provider_on_later = simple_usage(
        &counter,
        r#"[{"usage":{"completion_tokens":8}},{"_hidden_params":{"custom_llm_provider":"anthropic"},"usage":{"completion_tokens":0}}]"#,
    )
    .await;
    assert_eq!(provider_on_later.completion_tokens, 8);
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

    let response_without_choices = simple_usage(
        &counter,
        r#"[{"form":"model_response","_hidden_params":{"custom_llm_provider":"anthropic"},"usage":{"completion_tokens":8},"choices":[]}]"#,
    )
    .await;
    assert_eq!(response_without_choices.completion_tokens, 7);

    let response_choice = simple_usage(
        &counter,
        r#"[{"form":"model_response","_hidden_params":{"custom_llm_provider":"anthropic"},"usage":{"completion_tokens":8},"choices":[{}]}]"#,
    )
    .await;
    assert_eq!(response_choice.completion_tokens, 8);
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

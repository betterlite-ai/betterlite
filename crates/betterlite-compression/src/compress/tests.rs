#![cfg(test)]

use async_trait::async_trait;
use betterlite_tokenizer::count::error::CountError;
use indexmap::IndexMap;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Mutex;

use crate::compress::assembly::get_dropped_tool_span_indices;
use crate::compress::budget::{BudgetSelectionRequest, select_kept_indices_for_budget};
use crate::compress::counting::TokenCounter;
use crate::embedding_scoring::{EmbeddingCall, EmbeddingExecutor};
use crate::message_normalization::content_to_text;
use crate::truncate_message::truncate_message;

use super::*;

const MODEL: &str = "gpt-4";

#[derive(Clone, Debug, PartialEq)]
enum CountCall {
    Text { model: String, text: String },
    Messages { model: String, messages: Vec<Value> },
}

struct FakeCounter {
    message_totals: Mutex<VecDeque<usize>>,
    text_overrides: BTreeMap<String, usize>,
    calls: Mutex<Vec<CountCall>>,
}

fn fixture_counter(totals: Vec<usize>) -> FakeCounter {
    FakeCounter {
        message_totals: Mutex::new(VecDeque::from(totals)),
        text_overrides: BTreeMap::new(),
        calls: Mutex::new(Vec::new()),
    }
}

#[async_trait]
impl TokenCounter for FakeCounter {
    async fn count_text(&self, model: &str, text: &str) -> Result<usize, CountError> {
        self.calls
            .lock()
            .expect("recorded calls")
            .push(CountCall::Text {
                model: model.to_owned(),
                text: text.to_owned(),
            });
        if let Some(tokens) = self.text_overrides.get(text) {
            return Ok(*tokens);
        }
        Ok(text.split_whitespace().count())
    }

    async fn count_messages(&self, model: &str, messages: &[Value]) -> Result<usize, CountError> {
        self.calls
            .lock()
            .expect("recorded calls")
            .push(CountCall::Messages {
                model: model.to_owned(),
                messages: messages.to_vec(),
            });
        if let Some(total) = self
            .message_totals
            .lock()
            .expect("message totals")
            .pop_front()
        {
            return Ok(total);
        }
        let mut total = 4_usize.saturating_mul(messages.len());
        for message in messages {
            let text = message
                .get("content")
                .map(content_to_text)
                .unwrap_or_default();
            total = total.saturating_add(text.split_whitespace().count());
        }
        Ok(total)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the fake embedding call failed")]
struct FakeEmbeddingFailure;

struct RecordingExecutor {
    embeddings: Vec<Vec<f64>>,
    calls: Mutex<Vec<Map<String, Value>>>,
}

#[async_trait]
impl EmbeddingExecutor for RecordingExecutor {
    type Failure = FakeEmbeddingFailure;

    async fn embed(&self, call: EmbeddingCall<'_>) -> Result<Vec<Vec<f64>>, Self::Failure> {
        self.calls
            .lock()
            .expect("recorded embedding calls")
            .push(call.parameters.clone());
        Ok(self.embeddings.clone())
    }
}

fn words(count: usize) -> String {
    (0..count)
        .map(|index| format!("w{index}"))
        .collect::<Vec<String>>()
        .join(" ")
}

fn request<'request>(
    messages: &'request [Value],
    trigger: usize,
    target: Option<usize>,
) -> CompressRequest<'request> {
    CompressRequest {
        messages,
        model: MODEL,
        call_type: "completion",
        compression_trigger: Some(trigger),
        compression_target: target,
    }
}

mod compress_behavior {
    mod below_trigger {
        use super::super::*;
        use rstest::rstest;

        #[rstest]
        #[case::at_the_default_trigger(200_000_usize, true)]
        #[case::one_token_above_the_default(200_001, false)]
        #[tokio::test]
        async fn uses_two_hundred_thousand_tokens_when_none_is_supplied(
            #[case] original_tokens: usize,
            #[case] skipped: bool,
        ) {
            let messages = vec![json!({"role": "user", "content": "hello"})];
            let counter = fixture_counter(vec![original_tokens]);
            let compress_request = CompressRequest {
                messages: &messages,
                model: MODEL,
                call_type: "completion",
                compression_trigger: None,
                compression_target: None,
            };
            let result = compress(&compress_request, &counter)
                .await
                .expect("compressed");
            assert_eq!(result.compression_skipped_reason.is_some(), skipped);
        }

        #[tokio::test]
        async fn passes_messages_through_unchanged() {
            let messages = vec![
                json!({"role": "system", "content": "be helpful"}),
                json!({"role": "user", "content": "hello there"}),
            ];
            let counter = fixture_counter(vec![10]);
            let result = compress(&request(&messages, 10, None), &counter)
                .await
                .expect("compressed");
            assert_eq!(result.messages, messages);
            assert_eq!(result.original_tokens, 10);
            assert_eq!(result.compressed_tokens, 10);
            assert_eq!(result.compression_ratio.to_bits(), 0.0_f64.to_bits());
            assert!(result.cache.is_empty());
            assert!(result.tools.is_empty());
        }
    }

    mod skipped_reason {
        use super::super::*;

        #[tokio::test]
        async fn at_the_trigger_still_skips() {
            let messages = vec![json!({"role": "user", "content": "one two"})];
            let counter = fixture_counter(vec![10]);
            let result = compress(&request(&messages, 10, None), &counter)
                .await
                .expect("compressed");
            assert_eq!(
                result.compression_skipped_reason,
                Some(CompressionSkippedReason::BelowTrigger)
            );
            assert_eq!(
                CompressionSkippedReason::BelowTrigger.as_str(),
                "below_trigger"
            );
        }

        #[tokio::test]
        async fn above_the_trigger_compresses() {
            let messages = vec![
                json!({"role": "system", "content": "a b"}),
                json!({"role": "user", "content": "c d e f g h"}),
            ];
            let counter = fixture_counter(vec![11, 5]);
            let result = compress(&request(&messages, 10, None), &counter)
                .await
                .expect("compressed");
            assert_eq!(result.compression_skipped_reason, None);
        }
    }

    mod default_target {
        use super::super::*;
        use rstest::rstest;

        #[rstest]
        #[case::ten_token_trigger_fits(10_usize, 2_usize, true)]
        #[case::ten_token_trigger_drops(10, 3, false)]
        #[case::hundred_token_trigger_fits(100, 65, true)]
        #[case::hundred_token_trigger_drops(100, 66, false)]
        #[tokio::test]
        async fn is_seventy_percent_of_the_trigger(
            #[case] trigger: usize,
            #[case] candidate_words: usize,
            #[case] kept: bool,
        ) {
            let messages = vec![
                json!({"role": "system", "content": "s1 s2 s3"}),
                json!({"role": "user", "content": words(candidate_words)}),
                json!({"role": "assistant", "content": "a1"}),
                json!({"role": "user", "content": "u1"}),
            ];
            let counter = fixture_counter(vec![trigger + 1]);
            let result = compress(&request(&messages, trigger, None), &counter)
                .await
                .expect("compressed");
            if kept {
                assert_eq!(result.messages[1], messages[1]);
                assert!(result.cache.is_empty());
            } else {
                assert!(
                    result.messages[1]["content"]
                        .as_str()
                        .is_some_and(|text| text.starts_with("[Compressed:"))
                );
                assert_eq!(result.cache.len(), 1);
            }
        }
    }

    mod token_counting {
        use super::super::*;

        #[tokio::test]
        async fn counts_the_original_and_compressed_messages() {
            let messages = vec![
                json!({"role": "system", "content": "be brief"}),
                json!({"role": "user", "content": "one two three"}),
                json!({"role": "assistant", "content": "four five"}),
                json!({"role": "user", "content": "six seven"}),
            ];
            let counter = fixture_counter(vec![100, 40]);
            let result = compress(&request(&messages, 0, Some(0)), &counter)
                .await
                .expect("compressed");
            assert_eq!(result.original_tokens, 100);
            assert_eq!(result.compressed_tokens, 40);
            let calls = counter.calls.lock().expect("recorded calls").clone();
            let message_calls: Vec<CountCall> = calls
                .into_iter()
                .filter(|call| matches!(call, CountCall::Messages { .. }))
                .collect();
            assert_eq!(
                message_calls,
                vec![
                    CountCall::Messages {
                        model: MODEL.to_owned(),
                        messages: messages.clone(),
                    },
                    CountCall::Messages {
                        model: MODEL.to_owned(),
                        messages: result.messages.clone(),
                    },
                ]
            );
        }
    }

    mod compression_ratio {
        use super::super::*;
        use rstest::rstest;

        #[rstest]
        #[case::exact_fraction(1000_usize, 400_usize, 0.6_f64)]
        #[case::rounds_to_four_places(3, 2, 0.3333)]
        #[case::ties_round_to_even(32, 31, 0.0312)]
        #[case::grows_when_stubs_are_larger(10, 15, -0.5)]
        #[tokio::test]
        async fn is_one_minus_compressed_over_original(
            #[case] original_total: usize,
            #[case] compressed_total: usize,
            #[case] expected: f64,
        ) {
            let messages = vec![
                json!({"role": "system", "content": "s"}),
                json!({"role": "user", "content": "u"}),
            ];
            let counter = fixture_counter(vec![original_total, compressed_total]);
            let result = compress(&request(&messages, 0, Some(0)), &counter)
                .await
                .expect("compressed");
            assert_eq!(result.compression_ratio.to_bits(), expected.to_bits());
        }
    }

    mod protected_indices {
        use super::super::*;

        #[tokio::test]
        async fn keeps_system_last_user_and_last_assistant() {
            let messages = vec![
                json!({"role": "system", "content": "system instructions"}),
                json!({"role": "user", "content": "old question"}),
                json!({"role": "assistant", "content": "old answer"}),
                json!({"role": "user", "content": "new question"}),
            ];
            let counter = fixture_counter(vec![80, 60]);
            let result = compress(&request(&messages, 0, Some(0)), &counter)
                .await
                .expect("compressed");
            assert_eq!(result.messages.len(), 4);
            assert_eq!(result.messages[0], messages[0]);
            assert_eq!(result.messages[2], messages[2]);
            assert_eq!(result.messages[3], messages[3]);
            assert!(
                result.messages[1]["content"]
                    .as_str()
                    .is_some_and(|text| text.starts_with("[Compressed:"))
            );
            assert_eq!(
                result.cache.get("message_1").map(String::as_str),
                Some("old question")
            );
        }

        #[tokio::test]
        async fn keeps_every_message_up_to_the_last_cache_control() {
            let messages = vec![
                json!({"role": "system", "content": "system instructions"}),
                json!({"role": "user", "content": "pinned question"}),
                json!({"role": "assistant", "content": "pinned answer"}),
                json!({
                    "role": "user",
                    "content": "breakpoint question",
                    "cache_control": {"type": "ephemeral"},
                }),
                json!({"role": "user", "content": "middle question"}),
                json!({"role": "user", "content": "new question"}),
            ];
            let counter = fixture_counter(vec![80, 60]);
            let result = compress(&request(&messages, 0, Some(0)), &counter)
                .await
                .expect("compressed");
            assert_eq!(result.messages.len(), 6);
            assert_eq!(result.messages[0], messages[0]);
            assert_eq!(result.messages[1], messages[1]);
            assert_eq!(result.messages[2], messages[2]);
            assert_eq!(result.messages[3], messages[3]);
            assert_eq!(result.messages[5], messages[5]);
            assert!(
                result.messages[4]["content"]
                    .as_str()
                    .is_some_and(|text| text.starts_with("[Compressed:"))
            );
            assert_eq!(
                result.cache.get("message_4").map(String::as_str),
                Some("middle question")
            );
        }
    }

    mod invalid_anthropic {
        use super::super::*;

        fn broken_messages() -> Vec<Value> {
            vec![
                json!({"role": "user", "content": "hi"}),
                json!({"role": "assistant", "content": [
                    {"type": "tool_use", "id": "t1", "name": "lookup", "input": {}}
                ]}),
                json!({"role": "user", "content": [{"type": "text", "text": "never mind"}]}),
            ]
        }

        fn anthropic_request<'request>(messages: &'request [Value]) -> CompressRequest<'request> {
            CompressRequest {
                messages,
                model: MODEL,
                call_type: "anthropic_messages",
                compression_trigger: Some(0),
                compression_target: None,
            }
        }

        #[tokio::test]
        async fn returns_the_original_messages() {
            let messages = broken_messages();
            let counter = fixture_counter(vec![50]);
            let result = compress(&anthropic_request(&messages), &counter)
                .await
                .expect("compressed");
            assert_eq!(result.messages, messages);
            assert_eq!(result.original_tokens, 50);
            assert_eq!(result.compressed_tokens, 50);
            assert_eq!(result.compression_ratio.to_bits(), 0.0_f64.to_bits());
            assert!(result.cache.is_empty());
            assert!(result.tools.is_empty());
            assert_eq!(
                result.compression_skipped_reason,
                Some(CompressionSkippedReason::InvalidAnthropicToolSequence)
            );
            assert_eq!(
                CompressionSkippedReason::InvalidAnthropicToolSequence.as_str(),
                "invalid_anthropic_tool_sequence"
            );
        }

        #[tokio::test]
        async fn embeddings_are_scored_before_the_sequence_is_checked() {
            let messages = broken_messages();
            let counter = fixture_counter(vec![50]);
            let executor = RecordingExecutor {
                embeddings: vec![
                    vec![1.0_f64, 0.0],
                    vec![1.0, 0.0],
                    vec![0.0, 1.0],
                    vec![0.5, 0.5],
                ],
                calls: Mutex::new(Vec::new()),
            };
            let result = compress_with_embeddings(
                &anthropic_request(&messages),
                &counter,
                &EmbeddingScoring {
                    model: "text-embedding-3-small",
                    parameters: None,
                    cache_present: true,
                    executor: &executor,
                },
            )
            .await
            .expect("compressed");
            let calls = executor
                .calls
                .lock()
                .expect("recorded embedding calls")
                .clone();
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0]["caching"], json!(true));
            assert_eq!(calls[0]["input"].as_array().map(Vec::len), Some(4));
            assert_eq!(
                result.compression_skipped_reason,
                Some(CompressionSkippedReason::InvalidAnthropicToolSequence)
            );
            assert_eq!(result.messages, messages);
        }
    }

    mod target_override {
        use super::super::*;

        #[tokio::test]
        async fn protected_content_can_exceed_the_target() {
            let messages = vec![
                json!({"role": "system", "content": words(20)}),
                json!({"role": "user", "content": "question"}),
            ];
            let counter = fixture_counter(vec![200, 180]);
            let result = compress(&request(&messages, 0, Some(1)), &counter)
                .await
                .expect("compressed");
            assert_eq!(result.messages, messages);
            assert_eq!(result.compressed_tokens, 180);
            assert!(result.compressed_tokens > 1);
        }
    }
}

mod budget_selection {
    use super::super::*;

    mod token_counting {
        use super::super::*;
        use rstest::rstest;

        #[rstest]
        #[case::fits_exactly(110_usize, true)]
        #[case::one_token_short(109, false)]
        #[tokio::test]
        async fn counts_initial_kept_indices_against_the_target(
            #[case] target: usize,
            #[case] kept: bool,
        ) {
            let normalized = vec![json!({"content": words(60)}), json!({"content": words(50)})];
            let scores = vec![0.9_f64, 0.8];
            let counter = fixture_counter(Vec::new());
            let selection = select_kept_indices_for_budget(
                &BudgetSelectionRequest {
                    normalized_messages: &normalized,
                    original_messages: &normalized,
                    combined_scores: &scores,
                    compression_target: target,
                    model: MODEL,
                    initial_kept_indices: &[0],
                    tool_exchange_spans: &[],
                },
                &counter,
            )
            .await
            .expect("selected");
            assert_eq!(selection.kept_indices.contains(&1), kept);
            assert!(selection.kept_indices.contains(&0));
        }
    }

    mod preserves_protected {
        use super::super::*;

        #[tokio::test]
        async fn keeps_protected_indices_ahead_of_higher_scorers() {
            let normalized = vec![
                json!({"content": words(10)}),
                json!({"content": words(10)}),
                json!({"content": words(10)}),
            ];
            let scores = vec![0.1_f64, 0.99, 0.5];
            let counter = fixture_counter(Vec::new());
            let selection = select_kept_indices_for_budget(
                &BudgetSelectionRequest {
                    normalized_messages: &normalized,
                    original_messages: &normalized,
                    combined_scores: &scores,
                    compression_target: 10,
                    model: MODEL,
                    initial_kept_indices: &[0],
                    tool_exchange_spans: &[],
                },
                &counter,
            )
            .await
            .expect("selected");
            assert_eq!(selection.kept_indices, BTreeSet::from([0]));
        }
    }

    mod truncation {
        use super::super::*;
        use rstest::rstest;

        fn long_message() -> Value {
            let lines = (0_usize..20_usize)
                .map(|_line| {
                    (0_usize..10_usize)
                        .map(|word| format!("w{word}"))
                        .collect::<Vec<String>>()
                        .join(" ")
                })
                .collect::<Vec<String>>()
                .join("\n");
            json!({"role": "user", "content": lines})
        }

        #[rstest]
        #[case::hundred_tokens_left(100_usize, true)]
        #[case::ninety_nine_tokens_left(99, false)]
        #[tokio::test]
        async fn truncates_only_with_enough_budget_left(
            #[case] target: usize,
            #[case] truncated: bool,
        ) {
            let message = long_message();
            let normalized = vec![message.clone()];
            let scores = vec![0.9_f64];
            let counter = fixture_counter(Vec::new());
            let selection = select_kept_indices_for_budget(
                &BudgetSelectionRequest {
                    normalized_messages: &normalized,
                    original_messages: &normalized,
                    combined_scores: &scores,
                    compression_target: target,
                    model: MODEL,
                    initial_kept_indices: &[],
                    tool_exchange_spans: &[],
                },
                &counter,
            )
            .await
            .expect("selected");
            assert_eq!(selection.kept_indices.contains(&0), truncated);
            if truncated {
                let expected = truncate_message(&message, target);
                assert_ne!(expected["content"], message["content"]);
                assert_eq!(selection.truncated_overrides.get(&0), Some(&expected));
            } else {
                assert!(selection.truncated_overrides.is_empty());
            }
        }

        #[tokio::test]
        async fn never_truncates_a_tool_exchange() {
            let normalized = vec![json!({"content": words(60)}), json!({"content": words(60)})];
            let scores = vec![0.9_f64, 0.9];
            let spans = vec![vec![0_usize, 1]];
            let counter = fixture_counter(Vec::new());
            let selection = select_kept_indices_for_budget(
                &BudgetSelectionRequest {
                    normalized_messages: &normalized,
                    original_messages: &normalized,
                    combined_scores: &scores,
                    compression_target: 100,
                    model: MODEL,
                    initial_kept_indices: &[],
                    tool_exchange_spans: &spans,
                },
                &counter,
            )
            .await
            .expect("selected");
            assert!(selection.kept_indices.is_empty());
            assert!(selection.truncated_overrides.is_empty());
        }
    }

    #[tokio::test]
    async fn keeps_highest_scoring_units_first() {
        let normalized = vec![
            json!({"content": words(10)}),
            json!({"content": words(10)}),
            json!({"content": words(10)}),
        ];
        let scores = vec![0.1_f64, 0.9, 0.5];
        let counter = fixture_counter(Vec::new());
        let selection = select_kept_indices_for_budget(
            &BudgetSelectionRequest {
                normalized_messages: &normalized,
                original_messages: &normalized,
                combined_scores: &scores,
                compression_target: 25,
                model: MODEL,
                initial_kept_indices: &[],
                tool_exchange_spans: &[],
            },
            &counter,
        )
        .await
        .expect("selected");
        assert_eq!(selection.kept_indices, BTreeSet::from([1, 2]));
    }
}

mod tool_exchange_atomicity {
    use super::super::*;

    #[tokio::test]
    async fn keeps_both_messages_of_a_fitting_span() {
        let normalized = vec![
            json!({"content": "a a a"}),
            json!({"content": "b b"}),
            json!({"content": words(7)}),
        ];
        let scores = vec![0.1_f64, 0.9, 0.5];
        let spans = vec![vec![0_usize, 1]];
        let counter = fixture_counter(Vec::new());
        let selection = select_kept_indices_for_budget(
            &BudgetSelectionRequest {
                normalized_messages: &normalized,
                original_messages: &normalized,
                combined_scores: &scores,
                compression_target: 6,
                model: MODEL,
                initial_kept_indices: &[],
                tool_exchange_spans: &spans,
            },
            &counter,
        )
        .await
        .expect("selected");
        assert_eq!(selection.kept_indices, BTreeSet::from([0, 1]));
    }

    #[tokio::test]
    async fn never_truncates_a_span_that_does_not_fit() {
        let normalized = vec![
            json!({"content": words(60)}),
            json!({"content": words(60)}),
            json!({"content": "c"}),
        ];
        let scores = vec![0.9_f64, 0.9, 0.5];
        let spans = vec![vec![0_usize, 1]];
        let counter = fixture_counter(Vec::new());
        let selection = select_kept_indices_for_budget(
            &BudgetSelectionRequest {
                normalized_messages: &normalized,
                original_messages: &normalized,
                combined_scores: &scores,
                compression_target: 100,
                model: MODEL,
                initial_kept_indices: &[],
                tool_exchange_spans: &spans,
            },
            &counter,
        )
        .await
        .expect("selected");
        assert_eq!(selection.kept_indices, BTreeSet::from([2]));
        assert!(selection.truncated_overrides.is_empty());
    }

    #[tokio::test]
    async fn scores_a_span_by_its_best_member() {
        let normalized = vec![
            json!({"content": "a"}),
            json!({"content": "b"}),
            json!({"content": "c c c"}),
        ];
        let scores = vec![0.1_f64, 0.95, 0.5];
        let spans = vec![vec![0_usize, 1]];
        let counter = fixture_counter(Vec::new());
        let selection = select_kept_indices_for_budget(
            &BudgetSelectionRequest {
                normalized_messages: &normalized,
                original_messages: &normalized,
                combined_scores: &scores,
                compression_target: 3,
                model: MODEL,
                initial_kept_indices: &[],
                tool_exchange_spans: &spans,
            },
            &counter,
        )
        .await
        .expect("selected");
        assert_eq!(selection.kept_indices, BTreeSet::from([0, 1]));
    }

    #[tokio::test]
    async fn a_protected_member_keeps_the_whole_span() {
        let messages = vec![
            json!({"role": "user", "content": "start"}),
            json!({"role": "assistant", "content": [
                {"type": "tool_use", "id": "t1", "name": "lookup", "input": {},
                 "cache_control": {"type": "ephemeral"}}
            ]}),
            json!({"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "t1", "content": [{"type": "text", "text": "result"}]}
            ]}),
            json!({"role": "assistant", "content": "done"}),
            json!({"role": "user", "content": "now"}),
        ];
        let counter = fixture_counter(vec![90, 70]);
        let mut compress_request = request(&messages, 0, Some(0));
        compress_request.call_type = "anthropic_messages";
        let result = compress(&compress_request, &counter)
            .await
            .expect("compressed");
        assert_eq!(result.messages, messages);
        assert!(result.cache.is_empty());
    }
}

mod dropped_span_handling {
    use super::super::*;

    #[test]
    fn marks_every_index_of_a_dropped_span() {
        let kept = BTreeSet::from([0, 3]);
        let spans = vec![vec![1_usize, 2], vec![4, 5]];
        assert_eq!(
            get_dropped_tool_span_indices(&kept, &spans),
            BTreeSet::from([1, 2, 4, 5])
        );
        let partially_kept = BTreeSet::from([1]);
        assert!(get_dropped_tool_span_indices(&partially_kept, &spans[..1]).is_empty());
    }

    #[tokio::test]
    async fn omits_the_dropped_exchange_and_stubs_the_rest() {
        let messages = vec![
            json!({"role": "user", "content": "question one"}),
            json!({"role": "assistant", "content": [
                {"type": "tool_use", "id": "t1", "name": "lookup", "input": {}}
            ]}),
            json!({"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "t1", "content": [{"type": "text", "text": "result"}]}
            ]}),
            json!({"role": "assistant", "content": "middle answer"}),
            json!({"role": "user", "content": "question two"}),
        ];
        let counter = fixture_counter(vec![90, 20]);
        let mut compress_request = request(&messages, 0, Some(0));
        compress_request.call_type = "anthropic_messages";
        let result = compress(&compress_request, &counter)
            .await
            .expect("compressed");
        assert_eq!(result.messages.len(), 3);
        assert!(
            result.messages[0]["content"]
                .as_str()
                .is_some_and(|text| text.starts_with("[Compressed:"))
        );
        assert_eq!(result.messages[1], messages[3]);
        assert_eq!(result.messages[2], messages[4]);
        assert_eq!(
            result.cache,
            IndexMap::from([("message_0".to_owned(), "question one".to_owned())])
        );
        let keys = &result.tools[0]["input_schema"]["properties"]["key"]["enum"];
        assert_eq!(keys, &json!(["message_0"]));
    }
}

mod cache_key_extraction {
    use super::super::*;

    #[tokio::test]
    async fn maps_extracted_paths_to_the_full_content() {
        let messages = vec![
            json!({"role": "system", "content": "sys"}),
            json!({"role": "user", "content": "# main.py\nprint('hi')"}),
            json!({"role": "user", "content": "last question"}),
        ];
        let counter = fixture_counter(vec![40, 20]);
        let result = compress(&request(&messages, 0, Some(0)), &counter)
            .await
            .expect("compressed");
        assert_eq!(
            result.cache.get("main.py").map(String::as_str),
            Some("# main.py\nprint('hi')")
        );
    }

    #[tokio::test]
    async fn suffixes_duplicate_keys() {
        let messages = vec![
            json!({"role": "system", "content": "sys"}),
            json!({"role": "user", "content": "# main.py\nfirst"}),
            json!({"role": "user", "content": "# main.py\nsecond"}),
            json!({"role": "user", "content": "last question"}),
        ];
        let counter = fixture_counter(vec![40, 20]);
        let result = compress(&request(&messages, 0, Some(0)), &counter)
            .await
            .expect("compressed");
        let keys: Vec<&String> = result.cache.keys().collect();
        assert_eq!(keys, vec!["main.py", "main.py_2"]);
    }

    #[tokio::test]
    async fn falls_back_to_the_message_index() {
        let messages = vec![
            json!({"role": "system", "content": "sys"}),
            json!({"role": "user", "content": "no path here"}),
            json!({"role": "user", "content": "last question"}),
        ];
        let counter = fixture_counter(vec![40, 20]);
        let result = compress(&request(&messages, 0, Some(0)), &counter)
            .await
            .expect("compressed");
        assert_eq!(
            result.cache.get("message_1").map(String::as_str),
            Some("no path here")
        );
    }

    #[tokio::test]
    async fn uses_the_text_of_block_content() {
        let messages = vec![
            json!({"role": "system", "content": "sys"}),
            json!({"role": "user", "content": [
                {"type": "text", "text": "# app.ts"},
                {"type": "image", "source": {"type": "base64", "data": "AAAA"}}
            ]}),
            json!({"role": "user", "content": "last question"}),
        ];
        let counter = fixture_counter(vec![40, 20]);
        let result = compress(&request(&messages, 0, Some(0)), &counter)
            .await
            .expect("compressed");
        assert_eq!(
            result.cache.get("app.ts").map(String::as_str),
            Some("# app.ts")
        );
    }
}

mod retrieval_tool_schema {
    use super::super::*;

    #[tokio::test]
    async fn lists_every_cache_key_in_message_order() {
        let messages = vec![
            json!({"role": "system", "content": "sys"}),
            json!({"role": "user", "content": "first"}),
            json!({"role": "user", "content": "second"}),
            json!({"role": "user", "content": "last question"}),
        ];
        let counter = fixture_counter(vec![40, 20]);
        let result = compress(&request(&messages, 0, Some(0)), &counter)
            .await
            .expect("compressed");
        assert_eq!(result.tools.len(), 1);
        assert_eq!(result.tools[0]["type"], "function");
        assert_eq!(
            result.tools[0]["function"]["name"],
            crate::retrieval_tool::RETRIEVAL_TOOL_NAME
        );
        let keys = &result.tools[0]["function"]["parameters"]["properties"]["key"]["enum"];
        assert_eq!(keys, &json!(["message_1", "message_2"]));
    }

    #[tokio::test]
    async fn builds_no_tool_when_nothing_is_dropped() {
        let messages = vec![
            json!({"role": "system", "content": "sys"}),
            json!({"role": "user", "content": "last question"}),
        ];
        let counter = fixture_counter(vec![40, 20]);
        let result = compress(&request(&messages, 0, Some(1000)), &counter)
            .await
            .expect("compressed");
        assert!(result.cache.is_empty());
        assert!(result.tools.is_empty());
    }
}

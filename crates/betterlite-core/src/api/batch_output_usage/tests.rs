#![cfg(test)]

mod support;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use betterlite_models::decisions::Json;
use betterlite_models::usage_sum::Usage as SummedUsage;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use tracing::Level;

use super::decode::DecodedBatchLine;
use super::error::BatchOutputUsageError;
use super::tokens::{count_batch_entry_tokens, estimate_batch_entry_tokens};
use super::{
    BatchOutputDiagnostic, BatchOutputProvider, BatchOutputUsageInput, BatchOutputUsageResult,
    ConverseUsageOutcome, decode_batch_output_usage,
};
use crate::api::api_boundaries::ApiLimits;
use support::{
    BATCH_MODEL_INFO, ByModelLookup, EmbeddingProviders, FakeVertex, NoProviders, OCR_MODEL_INFO,
    RecordedCall, ScriptedCounter, bedrock_input, json_entry, logged, output_bytes, output_input,
    prompt_only_usage, run, run_captured, run_with_lookup, totals, vertex_input,
};

#[test]
fn provider_slugs_match_the_batch_providers() {
    assert_eq!(
        [
            BatchOutputProvider::Openai.slug(),
            BatchOutputProvider::Azure.slug(),
            BatchOutputProvider::VertexAi.slug(),
            BatchOutputProvider::HostedVllm.slug(),
            BatchOutputProvider::Anthropic.slug(),
            BatchOutputProvider::Bedrock.slug(),
            BatchOutputProvider::Mistral.slug(),
        ],
        [
            "openai",
            "azure",
            "vertex_ai",
            "hosted_vllm",
            "anthropic",
            "bedrock",
            "mistral",
        ]
    );
}

#[rstest::rstest]
#[case::success(
    r#"{"custom_id":"a","response":{"status_code":200,"body":{"model":"gpt-4o-batch","usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}}}"#,
    BatchOutputUsageResult {
        cost: Decimal::new(625, 2),
        prompt_cost: Decimal::new(5, 0),
        completion_cost: Decimal::new(125, 2),
        usage: totals(10, 5, 15),
        models: vec!["gpt-4o-batch".to_owned()],
        successful_requests: 1,
        failed_requests: 0,
        diagnostics: Vec::new(),
    }
)]
#[case::provider_failed(
    r#"{"custom_id":"b","response":{"status_code":400,"body":{"error":{"message":"bad"}}}}"#,
    BatchOutputUsageResult {
        cost: Decimal::ZERO,
        prompt_cost: Decimal::ZERO,
        completion_cost: Decimal::ZERO,
        usage: totals(0, 0, 0),
        models: Vec::new(),
        successful_requests: 0,
        failed_requests: 1,
        diagnostics: Vec::new(),
    }
)]
fn chat_output_usage(#[case] line: &str, #[case] expected: BatchOutputUsageResult) {
    let decoded = output_input("openai", None, Some(BATCH_MODEL_INFO), line);
    let result = run(&decoded, &NoProviders).expect("the batch is costed");
    assert_eq!(result, expected);
}

#[test]
fn chat_output_usage_counts_cached_tokens() {
    let decoded = output_input(
        "openai",
        Some("gpt-4o-batch"),
        Some(BATCH_MODEL_INFO),
        r#"{"custom_id": "a", "response": {"status_code": 200, "body": {"usage": {
            "prompt_tokens": 10,
            "completion_tokens": 5,
            "total_tokens": 15,
            "prompt_tokens_details": {"cached_tokens": 4, "cache_write_tokens": 6}
        }}}}"#,
    );
    let result = run(&decoded, &NoProviders).expect("the batch is costed");
    let details = result
        .usage
        .prompt_tokens_details
        .as_ref()
        .expect("the cached counters arrive as prompt details");
    assert_eq!(details.cached_tokens, Some(4));
    assert_eq!(details.cache_write_tokens, Some(6));
    assert_eq!(details.cache_creation_tokens, Some(6));
    assert_eq!(result.models, vec!["gpt-4o-batch".to_owned()]);
}

#[test]
fn a_cached_tokens_line_leaves_the_cache_write_detail_absent() {
    let decoded = output_input(
        "openai",
        Some("gpt-4o-batch"),
        Some(BATCH_MODEL_INFO),
        r#"{"custom_id": "read", "response": {"status_code": 200, "body": {"usage": {
            "prompt_tokens": 10,
            "completion_tokens": 5,
            "total_tokens": 15,
            "prompt_tokens_details": {"cached_tokens": 4}
        }}}}"#,
    );
    let result = run(&decoded, &NoProviders).expect("the batch is costed");
    let details = result
        .usage
        .prompt_tokens_details
        .as_ref()
        .expect("a positive cache-read counter creates the prompt details");
    assert_eq!(details.cached_tokens, Some(4));
    assert_eq!(details.cache_write_tokens, None);
    assert_eq!(details.cache_creation_tokens, None);
}

#[test]
fn a_cache_write_line_leaves_the_cached_tokens_detail_absent() {
    let decoded = output_input(
        "openai",
        Some("gpt-4o-batch"),
        Some(BATCH_MODEL_INFO),
        r#"{"custom_id": "write", "response": {"status_code": 200, "body": {"usage": {
            "prompt_tokens": 10,
            "completion_tokens": 5,
            "total_tokens": 15,
            "prompt_tokens_details": {"cache_write_tokens": 6}
        }}}}"#,
    );
    let result = run(&decoded, &NoProviders).expect("the batch is costed");
    let details = result
        .usage
        .prompt_tokens_details
        .as_ref()
        .expect("a positive cache-write counter creates the prompt details");
    assert_eq!(details.cached_tokens, None);
    assert_eq!(details.cache_write_tokens, Some(6));
    assert_eq!(details.cache_creation_tokens, Some(6));
}

#[test]
fn counts_reasoning_tokens_in_the_aggregate() {
    let decoded = output_input(
        "openai",
        Some("gpt-4o-batch"),
        Some(BATCH_MODEL_INFO),
        r#"{"custom_id": "r", "response": {"status_code": 200, "body": {"usage": {
            "prompt_tokens": 10,
            "completion_tokens": 5,
            "total_tokens": 15,
            "completion_tokens_details": {"reasoning_tokens": 7}
        }}}}"#,
    );
    let result = run(&decoded, &NoProviders).expect("the batch is costed");
    let details = result
        .usage
        .completion_tokens_details
        .as_ref()
        .expect("the reasoning counter arrives as completion details");
    assert_eq!(details.reasoning_tokens, Some(7));
    assert_eq!(details.text_tokens, Some(0));
    assert_eq!(result.usage.completion_tokens, 5);
    assert_eq!(result.usage.total_tokens, 15);
    assert_eq!(result.cost, Decimal::new(625, 2));
}

#[test]
fn a_non_object_response_slot_counts_failed() {
    let decoded = output_input(
        "openai",
        Some("gpt-4o-batch"),
        Some(BATCH_MODEL_INFO),
        r#"{"custom_id": "bad", "response": "not an object"},
        {"custom_id": "ok", "response": {"status_code": 200, "body": {"usage": {
            "prompt_tokens": 4,
            "completion_tokens": 0,
            "total_tokens": 4
        }}}}"#,
    );
    let result = run(&decoded, &NoProviders).expect("the batch is costed");
    assert_eq!(result.successful_requests, 1);
    assert_eq!(result.failed_requests, 1);
    assert_eq!(result.usage, totals(4, 0, 4));
    assert_eq!(result.cost, Decimal::new(2, 0));
}

#[test]
fn an_empty_response_model_is_no_model() {
    let decoded = output_input(
        "openai",
        None,
        Some(BATCH_MODEL_INFO),
        r#"{"custom_id": "e", "response": {"status_code": 200, "body": {
            "model": "",
            "usage": {"prompt_tokens": 4, "completion_tokens": 0, "total_tokens": 4}
        }}}"#,
    );
    let result = run(&decoded, &NoProviders).expect("the batch is costed");
    assert!(result.models.is_empty());
    assert_eq!(result.successful_requests, 1);
}

#[test]
fn line_cost_prefers_the_response_model() {
    let lookup = ByModelLookup {
        response: serde_json::from_str(r#"{"input_cost_per_token_batches":0.5}"#)
            .expect("the response rates decode"),
        deployment: serde_json::from_str(r#"{"input_cost_per_token_batches":0.25}"#)
            .expect("the deployment rates decode"),
    };
    let decoded = output_input(
        "openai",
        Some("gpt-4o-deployment"),
        None,
        r#"{"custom_id": "m", "response": {"status_code": 200, "body": {
            "model": "gpt-4o-response",
            "usage": {"prompt_tokens": 10, "completion_tokens": 0, "total_tokens": 10}
        }}}"#,
    );
    let result = run_with_lookup(&decoded, &lookup, &NoProviders).expect("the batch is costed");
    assert_eq!(result.prompt_cost, Decimal::new(5, 0));
    assert_eq!(result.completion_cost, Decimal::ZERO);
    assert_eq!(result.cost, Decimal::new(5, 0));
    assert_eq!(result.models, vec!["gpt-4o-deployment".to_owned()]);
}

#[test]
fn bedrock_prices_with_the_deployment_model() {
    let lookup = ByModelLookup {
        response: serde_json::from_str(r#"{"input_cost_per_token_batches":0.5}"#)
            .expect("the response rates decode"),
        deployment: serde_json::from_str(r#"{"input_cost_per_token_batches":0.25}"#)
            .expect("the deployment rates decode"),
    };
    let decoded = output_input(
        "bedrock",
        Some("gpt-4o-deployment"),
        None,
        r#"{"custom_id": "d", "modelOutput": {
            "model": "gpt-4o-response",
            "usage": {"input_tokens": 10, "output_tokens": 0}
        }}"#,
    );
    let result = run_with_lookup(&decoded, &lookup, &NoProviders).expect("the batch is costed");
    assert_eq!(result.prompt_cost, Decimal::new(25, 1));
    assert_eq!(result.cost, Decimal::new(25, 1));
}

#[test]
fn responses_output_usage() {
    let decoded = output_input(
        "openai",
        None,
        Some(BATCH_MODEL_INFO),
        r#"{"custom_id": "r", "response": {"status_code": 200, "body": {
            "model": "gpt-4o-batch",
            "usage": {"input_tokens": 12, "output_tokens": 8}
        }}}"#,
    );
    let result = run(&decoded, &NoProviders).expect("the batch is costed");
    assert_eq!(result.usage, totals(12, 8, 20));
    assert_eq!(result.prompt_cost, Decimal::new(6, 0));
    assert_eq!(result.completion_cost, Decimal::new(2, 0));
    assert_eq!(result.cost, Decimal::new(8, 0));
    assert_eq!(result.models, vec!["gpt-4o-batch".to_owned()]);
    assert_eq!(result.successful_requests, 1);
}

#[test]
fn anthropic_result_usage() {
    let decoded = output_input(
        "anthropic",
        None,
        Some(BATCH_MODEL_INFO),
        r#"{"custom_id": "a1", "result": {"type": "succeeded", "message": {
            "model": "claude-batch",
            "usage": {"input_tokens": 100, "output_tokens": 20}
        }}},
        {"custom_id": "a2", "result": {"type": "succeeded", "message": {
            "model": "claude-batch",
            "usage": {"mystery": 1}
        }}},
        {"custom_id": "a3", "result": {"type": "errored", "message": {
            "model": "claude-batch",
            "usage": {"input_tokens": 7, "output_tokens": 3}
        }}}"#,
    );
    let (outcome, logs) = run_captured(&decoded, &NoProviders);
    let result = outcome.expect("the batch is costed");
    assert_eq!(result.usage, totals(100, 20, 120));
    assert_eq!(result.prompt_cost, Decimal::new(50, 0));
    assert_eq!(result.completion_cost, Decimal::new(5, 0));
    assert_eq!(result.successful_requests, 2);
    assert_eq!(result.failed_requests, 1);
    assert_eq!(
        result.models,
        vec!["claude-batch".to_owned(), "claude-batch".to_owned()]
    );
    assert!(result.diagnostics.is_empty());
    assert!(logged(
        &logs,
        Level::WARN,
        "batch output line reported usage this parser does not understand"
    ));
}

#[test]
fn a_parsed_anthropic_usage_is_not_flagged() {
    let decoded = output_input(
        "anthropic",
        None,
        Some(BATCH_MODEL_INFO),
        r#"{"custom_id": "ok", "result": {"type": "succeeded", "message": {
            "model": "claude-batch",
            "usage": {"input_tokens": 4, "output_tokens": 1}
        }}}"#,
    );
    let (outcome, logs) = run_captured(&decoded, &NoProviders);
    let result = outcome.expect("the batch is costed");
    assert_eq!(result.usage, totals(4, 1, 5));
    assert!(!logged(
        &logs,
        Level::WARN,
        "batch output line reported usage this parser does not understand"
    ));
}

#[test]
fn an_anthropic_result_that_is_not_an_object_counts_failed() {
    let decoded = output_input(
        "anthropic",
        None,
        Some(BATCH_MODEL_INFO),
        r#"{"custom_id": "bad", "result": "succeeded"},
        {"custom_id": "ok", "result": {"type": "succeeded", "message": {
            "usage": {"input_tokens": 4, "output_tokens": 0}
        }}}"#,
    );
    let result = run(&decoded, &NoProviders).expect("the batch is costed");
    assert_eq!(result.successful_requests, 1);
    assert_eq!(result.failed_requests, 1);
    assert_eq!(result.usage, totals(4, 0, 4));
    assert_eq!(result.cost, Decimal::new(2, 0));
}

#[test]
fn ocr_line_cost() {
    let decoded = output_input(
        "mistral",
        Some("mistral-ocr-batch"),
        Some(OCR_MODEL_INFO),
        r#"{"custom_id": "o1", "response": {"status_code": 200, "body": {
            "usage_info": {"pages_processed": 3}
        }}},
        {"custom_id": "bad", "response": {"status_code": 200, "body": {
            "usage_info": {"pages_processed": "many"}
        }}}"#,
    );
    let result = run(&decoded, &NoProviders).expect("the batch is costed");
    assert_eq!(result.cost, Decimal::new(15, 1));
    assert_eq!(result.prompt_cost, Decimal::new(15, 1));
    assert_eq!(result.completion_cost, Decimal::ZERO);
    assert_eq!(result.usage, totals(0, 0, 0));
    assert_eq!(result.successful_requests, 2);
    assert_eq!(result.failed_requests, 0);
    assert!(result.diagnostics.is_empty());
}

#[test]
fn a_null_page_count_is_a_zero_page_ocr_line() {
    let decoded = output_input(
        "mistral",
        Some("mistral-ocr-batch"),
        Some(OCR_MODEL_INFO),
        r#"{"custom_id": "empty", "response": {"status_code": 200, "body": {
            "usage_info": {"pages_processed": null}
        }}}"#,
    );
    let (outcome, logs) = run_captured(&decoded, &NoProviders);
    let result = outcome.expect("the batch is costed");
    // LiteLLM's ocr_batch_cost reads a null page count as zero pages, so the line costs nothing without an error.
    assert_eq!(result.cost, Decimal::ZERO);
    assert_eq!(result.successful_requests, 1);
    assert_eq!(result.failed_requests, 0);
    assert!(!logged(
        &logs,
        Level::WARN,
        "batch output line could not be costed"
    ));
}

#[test]
fn ocr_annotation_pages_are_counted() {
    let decoded = output_input(
        "mistral",
        Some("mistral-ocr-batch"),
        Some(OCR_MODEL_INFO),
        r#"{"custom_id": "o2", "response": {"status_code": 200, "body": {
            "usage_info": {"pages_processed": 2, "pages_processed_annotation": 1, "credits": 3, "doc_size_bytes": 10}
        }}}"#,
    );
    let result = run(&decoded, &NoProviders).expect("the batch is costed");
    assert_eq!(result.cost, Decimal::new(15, 1));
    assert_eq!(result.successful_requests, 1);
}

#[test]
fn a_text_credit_count_is_an_uncostable_ocr_line() {
    let decoded = output_input(
        "mistral",
        Some("mistral-ocr-batch"),
        Some(OCR_MODEL_INFO),
        r#"{"custom_id": "c", "response": {"status_code": 200, "body": {
            "usage_info": {"pages_processed": 1, "credits": "3"}
        }}}"#,
    );
    let result = run(&decoded, &NoProviders).expect("the batch is costed");
    assert_eq!(result.cost, Decimal::ZERO);
    assert_eq!(result.successful_requests, 1);
}

#[rstest::rstest]
#[case::row_model_version(None, vec!["gemini-test".to_owned()], Decimal::new(625, 2))]
#[case::deployment_model(Some("deployment-model"), vec!["deployment-model".to_owned()], Decimal::new(625, 2))]
fn vertex_native_usage(
    #[case] model_name: Option<&str>,
    #[case] models: Vec<String>,
    #[case] cost: Decimal,
) {
    let decoded = vertex_input(
        model_name,
        Some(BATCH_MODEL_INFO),
        r#"{"request": {"contents": []}, "response": {"model": "gemini-test", "usage": {"prompt_tokens": 10, "completion_tokens": 5}}},
        {"request": {"contents": []}}"#,
    );
    let (outcome, logs) = run_captured(&decoded, &FakeVertex);
    let result = outcome.expect("the batch is costed");
    assert_eq!(result.usage, totals(10, 5, 15));
    assert_eq!(result.cost, cost);
    assert_eq!(result.models, models);
    assert_eq!(result.successful_requests, 1);
    assert_eq!(result.failed_requests, 1);
    assert!(result.diagnostics.is_empty());
    assert!(logged(&logs, Level::INFO, "vertex_ai batch cost"));
}

#[test]
fn vertex_models_are_counted_once() {
    let decoded = vertex_input(
        None,
        Some(BATCH_MODEL_INFO),
        r#"{"response": {"model": "gemini-test", "usage": {"prompt_tokens": 10, "completion_tokens": 5}}},
        {"response": {"model": "gemini-test", "usage": {"prompt_tokens": 10, "completion_tokens": 5}}}"#,
    );
    let result = run(&decoded, &FakeVertex).expect("the batch is costed");
    assert_eq!(result.usage, totals(20, 10, 30));
    assert_eq!(result.cost, Decimal::new(125, 1));
    assert_eq!(result.models, vec!["gemini-test".to_owned()]);
    assert_eq!(result.successful_requests, 2);
}

fn malformed_bytes() -> Vec<u8> {
    let good = r#"{"custom_id":"ok","response":{"status_code":200,"body":{"usage":{"prompt_tokens":4,"completion_tokens":0,"total_tokens":4}}}}"#;
    format!("not json\n42\n{good}\n").into_bytes()
}

#[test]
fn malformed_output_line_warning() {
    let decoded = output_bytes(
        "openai",
        Some("gpt-4o-batch"),
        Some(BATCH_MODEL_INFO),
        &malformed_bytes(),
    );
    let (outcome, logs) = run_captured(&decoded, &NoProviders);
    let result = outcome.expect("the batch is costed");
    assert_eq!(result.usage, totals(4, 0, 4));
    assert_eq!(result.cost, Decimal::new(2, 0));
    assert_eq!(result.successful_requests, 1);
    assert_eq!(result.failed_requests, 0);
    assert_eq!(
        result.diagnostics,
        vec![
            BatchOutputDiagnostic::MalformedLine,
            BatchOutputDiagnostic::NonObjectLine { kind: "int" },
        ]
    );
    assert!(logged(
        &logs,
        Level::WARN,
        "skipping malformed batch output line"
    ));
    assert!(logged(
        &logs,
        Level::WARN,
        "skipping non-object batch output line"
    ));
    assert!(logged(&logs, Level::DEBUG, "batch output aggregate"));
}

#[test]
fn an_uncostable_line_warns_and_stays_successful() {
    let decoded = output_input(
        "mistral",
        Some("mistral-ocr-batch"),
        Some(OCR_MODEL_INFO),
        r#"{"custom_id": "bad", "response": {"status_code": 200, "body": {
            "usage_info": {"pages_processed": "many"}
        }}}"#,
    );
    let (outcome, logs) = run_captured(&decoded, &NoProviders);
    let result = outcome.expect("the batch is costed");
    assert_eq!(result.cost, Decimal::ZERO);
    assert_eq!(result.successful_requests, 1);
    assert_eq!(result.failed_requests, 0);
    assert!(logged(
        &logs,
        Level::WARN,
        "batch output line could not be costed"
    ));
    assert!(logs.iter().any(|(level, message, custom_id)| {
        *level == Level::WARN
            && message.starts_with("batch output line could not be costed")
            && custom_id.contains("bad")
    }));
}

#[test]
fn empty_batch_is_a_zero_result() {
    let decoded = output_input("openai", None, None, "");
    let result = run(&decoded, &NoProviders).expect("the empty batch is costed");
    assert_eq!(result.cost, Decimal::ZERO);
    assert_eq!(result.usage, totals(0, 0, 0));
    assert!(result.models.is_empty());
    assert_eq!(result.successful_requests, 0);
    assert!(result.diagnostics.is_empty());
}

#[test]
fn decodes_bytes_with_python_byte_whitespace() {
    let bytes = b"\xef\xbb\xbf{\"custom_id\":\"a\"}\r\n\n  \n42\nnot json";
    let decoded =
        decode_batch_output_usage(bytes, &ApiLimits::default()).expect("the bytes decode");
    assert_eq!(decoded.len(), 3);
    assert_eq!(
        decoded[0],
        DecodedBatchLine::Entry(Json::Object(IndexMap::from([(
            "custom_id".to_owned(),
            Json::Text("a".to_owned()),
        )])))
    );
    assert_eq!(decoded[1], DecodedBatchLine::NonObject { kind: "int" });
    assert!(matches!(decoded[2], DecodedBatchLine::Malformed { .. }));
}

#[test]
fn rejects_lines_that_are_not_strict_utf8_json() {
    let invalid_utf8 = decode_batch_output_usage(b"\xff\xfe{\x00}\n", &ApiLimits::default())
        .expect("the bytes decode");
    assert!(matches!(
        invalid_utf8.as_slice(),
        [DecodedBatchLine::Malformed { .. }]
    ));
    let utf16 = decode_batch_output_usage(b"{\x00\"a\"\x00:\x001\x00}\x00", &ApiLimits::default())
        .expect("the bytes decode");
    assert!(matches!(
        utf16.as_slice(),
        [DecodedBatchLine::Malformed { .. }]
    ));
    let nan =
        decode_batch_output_usage(b"{\"a\":NaN}", &ApiLimits::default()).expect("the bytes decode");
    assert!(matches!(
        nan.as_slice(),
        [DecodedBatchLine::Malformed { .. }]
    ));
}

#[test]
fn decodes_preparsed_lines() {
    let decoded = output_input("openai", None, None, r#"{"custom_id": "a"}, 7, 1.5"#);
    let BatchOutputUsageInput::OutputUsage(request) = &decoded else {
        panic!("the fixture is an output usage request");
    };
    let lines = super::decode::decoded_lines(&request.content, &ApiLimits::default())
        .expect("the lines decode");
    assert_eq!(lines.len(), 3);
    assert!(matches!(lines[0], DecodedBatchLine::Entry(_)));
    assert_eq!(lines[1], DecodedBatchLine::NonObject { kind: "int" });
    assert_eq!(lines[2], DecodedBatchLine::NonObject { kind: "float" });
}

#[test]
fn rejects_a_batch_output_over_the_json_limit() {
    let limits = ApiLimits {
        max_json_bytes: 3,
        max_event_bytes: 1024,
        max_items: 16,
    };
    let error = decode_batch_output_usage(b"1234", &limits).expect_err("the batch is too large");
    assert!(matches!(error, BatchOutputUsageError::JsonTooLarge(4, 3)));
}

#[test]
fn rejects_a_line_over_the_event_limit() {
    let limits = ApiLimits {
        max_json_bytes: 1024,
        max_event_bytes: 4,
        max_items: 16,
    };
    let error = decode_batch_output_usage(b"12345", &limits).expect_err("the line is too large");
    assert!(matches!(error, BatchOutputUsageError::LineTooLarge(5, 4)));
}

#[test]
fn rejects_more_lines_than_the_item_limit() {
    let limits = ApiLimits {
        max_json_bytes: 1024,
        max_event_bytes: 1024,
        max_items: 1,
    };
    let error = decode_batch_output_usage(b"1\n2", &limits).expect_err("there are too many lines");
    assert!(matches!(error, BatchOutputUsageError::TooManyLines(1)));
}

#[test]
fn accepts_a_batch_at_the_json_limit() {
    let limits = ApiLimits {
        max_json_bytes: 5,
        max_event_bytes: 1024,
        max_items: 16,
    };
    let decoded = decode_batch_output_usage(b"1\n2\n3", &limits).expect("the batch fits");
    assert_eq!(decoded.len(), 3);
}

#[test]
fn accepts_a_line_at_the_event_limit() {
    let limits = ApiLimits {
        max_json_bytes: 1024,
        max_event_bytes: 2,
        max_items: 16,
    };
    let decoded = decode_batch_output_usage(b"12", &limits).expect("the line fits");
    assert_eq!(decoded.len(), 1);
}

#[test]
fn replays_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/batch_output_usage");
    let seeds: Vec<PathBuf> = fs::read_dir(corpus)
        .expect("the corpus folder reads")
        .map(|entry| entry.expect("the corpus entry reads").path())
        .collect();
    assert!(!seeds.is_empty(), "the corpus has seeds");
    let empty = seeds
        .iter()
        .filter(|path| {
            let bytes = fs::read(path).expect("the seed reads");
            decode_batch_output_usage(&bytes, &ApiLimits::default())
                .expect("the seed decodes")
                .is_empty()
        })
        .count();
    assert_eq!(empty, 0, "every seed decodes to at least one line");
}

#[test]
fn estimates_tokens_from_serialized_size() {
    assert_eq!(estimate_batch_entry_tokens(b""), 1);
    assert_eq!(estimate_batch_entry_tokens(b"abc"), 1);
    assert_eq!(estimate_batch_entry_tokens(b"12345678"), 2);
}

#[tokio::test]
async fn counts_a_prompt_string_through_the_counter() {
    let counter = ScriptedCounter::with_answer(3);
    let entry = json_entry(r#"{"body": {"model": "gpt", "prompt": "hello"}}"#);
    let tokens = count_batch_entry_tokens(&entry, Some("fallback"), &counter)
        .await
        .expect("the entry counts");
    assert_eq!(tokens, 3);
    assert_eq!(
        counter.taken(),
        vec![RecordedCall {
            model: "gpt".to_owned(),
            text: Some("hello".to_owned()),
            messages: None,
            count_response_tokens: Some(false),
        }]
    );
}

#[tokio::test]
async fn counts_prompt_chunks_that_are_tokens_or_text() {
    let counter = ScriptedCounter::with_answer(3);
    let entry = json_entry(r#"{"body": {"prompt": ["a", 1, 2, true, 1.5, ["b", 3, 2.5]]}}"#);
    let tokens = count_batch_entry_tokens(&entry, Some("fallback"), &counter)
        .await
        .expect("the entry counts");
    // The two floats are not integer tokens, so they add nothing.
    assert_eq!(tokens, 3 + 1 + 1 + 1 + 3 + 1);
    let calls = counter.taken();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].model, "fallback");
    assert_eq!(calls[1].text, Some("b".to_owned()));
}

#[tokio::test]
async fn a_deeper_prompt_list_adds_nothing() {
    let counter = ScriptedCounter::with_answer(3);
    let entry = json_entry(r#"{"body": {"prompt": [["a", ["b"]], 1]}}"#);
    let tokens = count_batch_entry_tokens(&entry, Some("fallback"), &counter)
        .await
        .expect("the entry counts");
    // LiteLLM counts the strings and integers of one pre-tokenized chunk and ignores a list nested inside it.
    assert_eq!(tokens, 3 + 1);
    let calls = counter.taken();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].text, Some("a".to_owned()));
}

#[tokio::test]
async fn counts_messages_with_the_counter() {
    let counter = ScriptedCounter::with_answer(4);
    let entry = json_entry(
        r#"{"body": {
            "model": "gpt",
            "messages": [{"role": "user", "content": "hi"}]
        }}"#,
    );
    let tokens = count_batch_entry_tokens(&entry, None, &counter)
        .await
        .expect("the entry counts");
    assert_eq!(tokens, 4);
    assert_eq!(
        counter.taken(),
        vec![RecordedCall {
            model: "gpt".to_owned(),
            text: None,
            messages: Some(1),
            count_response_tokens: Some(false),
        }]
    );
}

#[tokio::test]
async fn an_entry_without_a_prompt_counts_zero() {
    let counter = ScriptedCounter::with_answer(3);
    let entry = json_entry(r#"{"body": {"model": "gpt"}}"#);
    let tokens = count_batch_entry_tokens(&entry, None, &counter)
        .await
        .expect("the entry counts");
    assert_eq!(tokens, 0);
    assert!(counter.taken().is_empty());
}

#[tokio::test]
async fn rejects_a_body_that_is_not_an_object() {
    let counter = ScriptedCounter::with_answer(3);
    let entry = json_entry(r#"{"body": "text"}"#);
    let body_error = count_batch_entry_tokens(&entry, None, &counter)
        .await
        .expect_err("the body is not an object");
    assert!(matches!(
        body_error,
        BatchOutputUsageError::NotAnObject { field: "body" }
    ));
    let entry_error = count_batch_entry_tokens(&Json::List(Vec::new()), None, &counter)
        .await
        .expect_err("the entry is not an object");
    assert!(matches!(entry_error, BatchOutputUsageError::NotAnEntry));
}

#[test]
fn bedrock_prefers_embedding_usage_over_the_usage_object() {
    let providers = EmbeddingProviders {
        embedding: Some(prompt_only_usage(7)),
        converse: ConverseUsageOutcome::Usage(Box::new(prompt_only_usage(99))),
    };
    let result = run(&bedrock_input(r#"{"inputTextTokenCount": 5}"#), &providers)
        .expect("the batch is costed");
    assert_eq!(result.usage.prompt_tokens, 7);
    assert_eq!(result.usage.total_tokens, 7);
}

#[test]
fn bedrock_uses_converse_usage_before_the_anthropic_shape() {
    let providers = EmbeddingProviders {
        embedding: None,
        converse: ConverseUsageOutcome::Usage(Box::new(SummedUsage {
            prompt_tokens: 3,
            completion_tokens: 2,
            total_tokens: 5,
            prompt_tokens_details: None,
            completion_tokens_details: None,
            numeric_extras: BTreeMap::new(),
        })),
    };
    let result = run(
        &bedrock_input(r#"{"inputTokens": 3, "outputTokens": 2}"#),
        &providers,
    )
    .expect("the batch is costed");
    assert_eq!(result.usage, totals(3, 2, 5));
}

#[test]
fn bedrock_falls_back_to_the_anthropic_usage_shape() {
    let providers = EmbeddingProviders {
        embedding: None,
        converse: ConverseUsageOutcome::NotConverse,
    };
    let result = run(
        &bedrock_input(r#"{"input_tokens": 4, "output_tokens": 1}"#),
        &providers,
    )
    .expect("the batch is costed");
    assert_eq!(result.usage, totals(4, 1, 5));
}

#[test]
fn bedrock_success_needs_model_output_and_no_error() {
    let decoded = output_input(
        "bedrock",
        Some("bedrock-batch"),
        Some(BATCH_MODEL_INFO),
        r#"{"custom_id": "ok", "modelOutput": {"usage": {"input_tokens": 4, "output_tokens": 1}}},
        {"custom_id": "errored", "error": {"message": "boom"}, "modelOutput": {
            "usage": {"input_tokens": 9, "output_tokens": 9}
        }},
        {"custom_id": "empty"}"#,
    );
    let result = run(&decoded, &NoProviders).expect("the batch is costed");
    assert_eq!(result.usage, totals(4, 1, 5));
    assert_eq!(result.successful_requests, 1);
    assert_eq!(result.failed_requests, 2);
}

#[test]
fn an_unreadable_converse_usage_is_uncostable() {
    let providers = EmbeddingProviders {
        embedding: None,
        converse: ConverseUsageOutcome::Invalid,
    };
    let result =
        run(&bedrock_input(r#"{"inputTokens": "x"}"#), &providers).expect("the batch is costed");
    assert_eq!(result.cost, Decimal::ZERO);
    assert_eq!(result.successful_requests, 1);
    assert!(result.diagnostics.is_empty());
}

#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;
use std::fs;
use std::mem;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_models::token_rates::ModelInfo;
use betterlite_models::usage_sum::Usage as SummedUsage;
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::{Message, TokenRequest};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use super::decode::DecodedBatchLine;
use super::error::BatchOutputUsageError;
use super::pricing::BatchRowPricer;
use super::tokens::{BatchTokenCounter, count_batch_entry_tokens, estimate_batch_entry_tokens};
use super::{
    BatchOutputDiagnostic, BatchOutputProvider, BatchOutputProviders, BatchOutputUsage,
    BatchOutputUsageInput, BatchOutputUsageResult, BatchUsageTotals, ConverseUsageOutcome,
    VertexRowStats, decode_batch_output_usage,
};
use crate::api::api_boundaries::ApiLimits;

// Every case supplies its own prices, so the published table never answers here.
struct NoLookup;

impl PriceLookup for NoLookup {
    fn effective_model_info(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        None
    }

    fn raw_cost_map_entry(&self, _key: &str) -> Option<PriceRecord> {
        None
    }

    fn effective_priced_model(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PricedModel> {
        None
    }
}

struct NoProviders;

impl BatchOutputProviders for NoProviders {
    fn bedrock_embedding_usage(
        &self,
        _response_body: &IndexMap<String, Json>,
    ) -> Option<SummedUsage> {
        None
    }

    fn bedrock_converse_usage(&self, _usage: &Json) -> ConverseUsageOutcome {
        ConverseUsageOutcome::NotConverse
    }

    fn vertex_row_stats(
        &self,
        _row: &Json,
        _model_name: Option<&str>,
        _model_info: Option<&ModelInfo>,
        _pricer: &dyn BatchRowPricer,
    ) -> Option<VertexRowStats> {
        None
    }
}

struct ByModelLookup {
    response: PriceRecord,
    deployment: PriceRecord,
}

impl PriceLookup for ByModelLookup {
    fn effective_model_info(
        &self,
        model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        match model {
            "gpt-4o-response" => Some(self.response.clone()),
            "gpt-4o-deployment" => Some(self.deployment.clone()),
            _ => None,
        }
    }

    fn raw_cost_map_entry(&self, _key: &str) -> Option<PriceRecord> {
        None
    }

    fn effective_priced_model(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PricedModel> {
        None
    }
}

fn run(
    input: &BatchOutputUsageInput,
    providers: &dyn BatchOutputProviders,
) -> Result<BatchOutputUsageResult, BatchOutputUsageError> {
    run_with_lookup(input, &NoLookup, providers)
}

fn run_with_lookup(
    input: &BatchOutputUsageInput,
    lookup: &dyn PriceLookup,
    providers: &dyn BatchOutputProviders,
) -> Result<BatchOutputUsageResult, BatchOutputUsageError> {
    let catalog = ModelCatalog::default();
    let names = BTreeSet::new();
    let environment = BTreeMap::new();
    let resolution = ResolutionContext {
        catalog: &catalog,
        known_model_names: &names,
        custom_providers: &names,
        environment: &environment,
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let service = BatchOutputUsage {
        limits: ApiLimits::default(),
        lookup,
        resolution: &resolution,
        providers,
    };
    service.run(input)
}

fn input(value: Value) -> BatchOutputUsageInput {
    serde_json::from_value(value).expect("the fixture input decodes")
}

fn json_entry(value: Value) -> Json {
    serde_json::from_value(value).expect("the batch entry decodes")
}

fn batch_model_info() -> ModelInfo {
    serde_json::from_str(
        r#"{"input_cost_per_token_batches":0.5,"output_cost_per_token_batches":0.25}"#,
    )
    .expect("the batch rates decode")
}

fn ocr_model_info() -> ModelInfo {
    serde_json::from_str(r#"{"ocr_cost_per_page_batches":0.5}"#).expect("the OCR rates decode")
}

const fn totals(prompt_tokens: i64, completion_tokens: i64, total_tokens: i64) -> BatchUsageTotals {
    BatchUsageTotals {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        reasoning_tokens: 0,
        cache_read_tokens: 0,
        cache_creation_tokens: 0,
    }
}

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
    let decoded = input(json!({
        "kind": "output_usage",
        "provider": "openai",
        "model_info": batch_model_info(),
        "content": {"kind": "lines", "lines": [serde_json::from_str::<Value>(line).expect("the line decodes")]},
    }));
    let result = run(&decoded, &NoProviders).expect("the batch is costed");
    assert_eq!(result, expected);
}

#[test]
fn chat_output_usage_counts_cached_tokens() {
    let decoded = input(json!({
        "kind": "output_usage",
        "provider": "openai",
        "model_name": "gpt-4o-batch",
        "model_info": batch_model_info(),
        "content": {"kind": "lines", "lines": [{
            "custom_id": "a",
            "response": {"status_code": 200, "body": {"usage": {
                "prompt_tokens": 10,
                "completion_tokens": 5,
                "total_tokens": 15,
                "prompt_tokens_details": {"cached_tokens": 4, "cache_write_tokens": 6},
            }}},
        }]},
    }));
    let result = run(&decoded, &NoProviders).expect("the batch is costed");
    assert_eq!(result.usage.cache_read_tokens, 4);
    assert_eq!(result.usage.cache_creation_tokens, 6);
    assert_eq!(result.models, vec!["gpt-4o-batch".to_owned()]);
}

#[test]
fn line_cost_prefers_the_response_model() {
    let lookup = ByModelLookup {
        response: serde_json::from_str(r#"{"input_cost_per_token_batches":0.5}"#)
            .expect("the response rates decode"),
        deployment: serde_json::from_str(r#"{"input_cost_per_token_batches":0.25}"#)
            .expect("the deployment rates decode"),
    };
    let decoded = input(json!({
        "kind": "output_usage",
        "provider": "openai",
        "model_name": "gpt-4o-deployment",
        "content": {"kind": "lines", "lines": [{
            "custom_id": "m",
            "response": {"status_code": 200, "body": {
                "model": "gpt-4o-response",
                "usage": {"prompt_tokens": 10, "completion_tokens": 0, "total_tokens": 10},
            }},
        }]},
    }));
    let result = run_with_lookup(&decoded, &lookup, &NoProviders).expect("the batch is costed");
    assert_eq!(result.prompt_cost, Decimal::new(5, 0));
    assert_eq!(result.completion_cost, Decimal::ZERO);
    assert_eq!(result.cost, Decimal::new(5, 0));
}

#[test]
fn responses_output_usage() {
    let decoded = input(json!({
        "kind": "output_usage",
        "provider": "openai",
        "model_info": batch_model_info(),
        "content": {"kind": "lines", "lines": [{
            "custom_id": "r",
            "response": {"status_code": 200, "body": {
                "model": "gpt-4o-batch",
                "usage": {"input_tokens": 12, "output_tokens": 8},
            }},
        }]},
    }));
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
    let decoded = input(json!({
        "kind": "output_usage",
        "provider": "anthropic",
        "model_info": batch_model_info(),
        "content": {"kind": "lines", "lines": [
            {"custom_id": "a1", "result": {"type": "succeeded", "message": {
                "model": "claude-batch",
                "usage": {"input_tokens": 100, "output_tokens": 20},
            }}},
            {"custom_id": "a2", "result": {"type": "succeeded", "message": {
                "model": "claude-batch",
                "usage": {"mystery": 1},
            }}},
            {"custom_id": "a3", "result": {"type": "errored", "message": {
                "model": "claude-batch",
                "usage": {"input_tokens": 7, "output_tokens": 3},
            }}},
        ]},
    }));
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
    let decoded = input(json!({
        "kind": "output_usage",
        "provider": "anthropic",
        "model_info": batch_model_info(),
        "content": {"kind": "lines", "lines": [
            {"custom_id": "ok", "result": {"type": "succeeded", "message": {
                "model": "claude-batch",
                "usage": {"input_tokens": 4, "output_tokens": 1},
            }}},
        ]},
    }));
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
fn ocr_line_cost() {
    let decoded = input(json!({
        "kind": "output_usage",
        "provider": "mistral",
        "model_name": "mistral-ocr-batch",
        "model_info": ocr_model_info(),
        "content": {"kind": "lines", "lines": [
            {"custom_id": "o1", "response": {"status_code": 200, "body": {
                "usage_info": {"pages_processed": 3},
            }}},
            {"custom_id": "bad", "response": {"status_code": 200, "body": {
                "usage_info": {"pages_processed": "many"},
            }}},
        ]},
    }));
    let result = run(&decoded, &NoProviders).expect("the batch is costed");
    assert_eq!(result.cost, Decimal::new(15, 1));
    assert_eq!(result.prompt_cost, Decimal::new(15, 1));
    assert_eq!(result.completion_cost, Decimal::ZERO);
    assert_eq!(result.usage, totals(0, 0, 0));
    assert_eq!(result.successful_requests, 2);
    assert_eq!(result.failed_requests, 0);
    assert!(result.diagnostics.is_empty());
}

fn row_stats(
    body: &Json,
    model_name: Option<&str>,
    model_info: Option<&ModelInfo>,
    pricer: &dyn BatchRowPricer,
) -> Option<VertexRowStats> {
    let Json::Object(fields) = body else {
        return None;
    };
    let Some(Json::Object(response)) = fields.get("response") else {
        return None;
    };
    let Some(Json::Object(usage_fields)) = response.get("usage") else {
        return None;
    };
    let prompt_tokens = count_of(usage_fields.get("prompt_tokens"));
    let completion_tokens = count_of(usage_fields.get("completion_tokens"));
    let model = model_name.map_or_else(
        || super::usage::response_model(Some(response)),
        |name| Some(name.to_owned()),
    );
    let usage = SummedUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens: prompt_tokens.saturating_add(completion_tokens),
        prompt_tokens_details: None,
        completion_tokens_details: None,
        numeric_extras: BTreeMap::new(),
    };
    Some(priced_row(usage, model, model_info, pricer))
}

fn priced_row(
    usage: SummedUsage,
    model: Option<String>,
    model_info: Option<&ModelInfo>,
    pricer: &dyn BatchRowPricer,
) -> VertexRowStats {
    let cost = model
        .as_deref()
        .and_then(|name| pricer.price(&usage, name, "vertex_ai", model_info).ok());
    let total_tokens = usage.total_tokens;
    VertexRowStats {
        usage,
        total_tokens,
        model,
        prompt_cost: cost.map_or(Decimal::ZERO, |cost| cost.prompt_cost),
        completion_cost: cost.map_or(Decimal::ZERO, |cost| cost.completion_cost),
    }
}

fn count_of(value: Option<&Json>) -> i64 {
    match value {
        Some(Json::Number(number)) => number.as_i64().unwrap_or_default(),
        Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_))
        | None => 0,
    }
}

struct FakeVertex;

impl BatchOutputProviders for FakeVertex {
    fn bedrock_embedding_usage(
        &self,
        _response_body: &IndexMap<String, Json>,
    ) -> Option<SummedUsage> {
        None
    }

    fn bedrock_converse_usage(&self, _usage: &Json) -> ConverseUsageOutcome {
        ConverseUsageOutcome::NotConverse
    }

    fn vertex_row_stats(
        &self,
        row: &Json,
        model_name: Option<&str>,
        model_info: Option<&ModelInfo>,
        pricer: &dyn BatchRowPricer,
    ) -> Option<VertexRowStats> {
        row_stats(row, model_name, model_info, pricer)
    }
}

#[rstest::rstest]
#[case::row_model_version(None, vec!["gemini-test".to_owned()], Decimal::new(625, 2))]
#[case::deployment_model(Some("deployment-model"), vec!["deployment-model".to_owned()], Decimal::new(625, 2))]
fn vertex_native_usage(
    #[case] model_name: Option<&str>,
    #[case] models: Vec<String>,
    #[case] cost: Decimal,
) {
    let decoded = input(json!({
        "kind": "vertex_usage",
        "model_name": model_name,
        "model_info": batch_model_info(),
        "content": {"kind": "lines", "lines": [
            {"request": {"contents": []}, "response": {"model": "gemini-test", "usage": {"prompt_tokens": 10, "completion_tokens": 5}}},
            {"request": {"contents": []}},
        ]},
    }));
    let result = run(&decoded, &FakeVertex).expect("the batch is costed");
    assert_eq!(result.usage, totals(10, 5, 15));
    assert_eq!(result.cost, cost);
    assert_eq!(result.models, models);
    assert_eq!(result.successful_requests, 1);
    assert_eq!(result.failed_requests, 1);
    assert!(result.diagnostics.is_empty());
}

#[test]
fn vertex_models_are_counted_once() {
    let decoded = input(json!({
        "kind": "vertex_usage",
        "model_info": batch_model_info(),
        "content": {"kind": "lines", "lines": [
            {"response": {"model": "gemini-test", "usage": {"prompt_tokens": 10, "completion_tokens": 5}}},
            {"response": {"model": "gemini-test", "usage": {"prompt_tokens": 10, "completion_tokens": 5}}},
        ]},
    }));
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
    let decoded = input(json!({
        "kind": "output_usage",
        "provider": "openai",
        "model_name": "gpt-4o-batch",
        "model_info": batch_model_info(),
        "content": {"kind": "bytes", "bytes": malformed_bytes()},
    }));
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
}

#[test]
fn an_uncostable_line_warns_and_stays_successful() {
    let decoded = input(json!({
        "kind": "output_usage",
        "provider": "mistral",
        "model_name": "mistral-ocr-batch",
        "model_info": ocr_model_info(),
        "content": {"kind": "lines", "lines": [
            {"custom_id": "bad", "response": {"status_code": 200, "body": {
                "usage_info": {"pages_processed": "many"},
            }}},
        ]},
    }));
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
    let decoded = input(json!({
        "kind": "output_usage",
        "provider": "openai",
        "content": {"kind": "lines", "lines": []},
    }));
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
fn decodes_preparsed_lines() {
    let decoded = input(json!({
        "kind": "output_usage",
        "provider": "openai",
        "content": {"kind": "lines", "lines": [{"custom_id": "a"}, 7, 1.5]},
    }));
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

#[derive(Debug, PartialEq, Eq)]
struct RecordedCall {
    model: String,
    text: Option<String>,
    messages: Option<usize>,
    count_response_tokens: Option<bool>,
}

#[derive(Default)]
struct ScriptedCounter {
    answer: usize,
    calls: Mutex<Vec<RecordedCall>>,
}

impl ScriptedCounter {
    const fn with_answer(answer: usize) -> Self {
        Self {
            answer,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn taken(&self) -> Vec<RecordedCall> {
        match self.calls.lock() {
            Ok(mut calls) => mem::take(&mut calls),
            Err(poisoned) => mem::take(&mut poisoned.into_inner()),
        }
    }
}

#[async_trait::async_trait]
impl BatchTokenCounter for ScriptedCounter {
    async fn count_tokens(&self, request: &TokenRequest<'_>) -> Result<usize, CountError> {
        let text = request.text.map(|text| text.joined().into_owned());
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedCall {
                model: request.model.to_owned(),
                text,
                messages: request.messages.map(<[Message]>::len),
                count_response_tokens: request.count_response_tokens,
            });
        Ok(self.answer)
    }
}

#[tokio::test]
async fn counts_a_prompt_string_through_the_counter() {
    let counter = ScriptedCounter::with_answer(3);
    let entry = json_entry(json!({"body": {"model": "gpt", "prompt": "hello"}}));
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
    let entry = json_entry(json!({"body": {"prompt": ["a", 1, 2, true, 1.5, ["b", 3, 2.5]]}}));
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
async fn counts_messages_with_the_counter() {
    let counter = ScriptedCounter::with_answer(4);
    let entry = json_entry(json!({"body": {
        "model": "gpt",
        "messages": [{"role": "user", "content": "hi"}],
    }}));
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
    let entry = json_entry(json!({"body": {"model": "gpt"}}));
    let tokens = count_batch_entry_tokens(&entry, None, &counter)
        .await
        .expect("the entry counts");
    assert_eq!(tokens, 0);
    assert!(counter.taken().is_empty());
}

#[tokio::test]
async fn rejects_a_body_that_is_not_an_object() {
    let counter = ScriptedCounter::with_answer(3);
    let entry = json_entry(json!({"body": "text"}));
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

struct EmbeddingProviders {
    embedding: Option<SummedUsage>,
    converse: ConverseUsageOutcome,
}

impl BatchOutputProviders for EmbeddingProviders {
    fn bedrock_embedding_usage(
        &self,
        _response_body: &IndexMap<String, Json>,
    ) -> Option<SummedUsage> {
        self.embedding.clone()
    }

    fn bedrock_converse_usage(&self, _usage: &Json) -> ConverseUsageOutcome {
        self.converse.clone()
    }

    fn vertex_row_stats(
        &self,
        _row: &Json,
        _model_name: Option<&str>,
        _model_info: Option<&ModelInfo>,
        _pricer: &dyn BatchRowPricer,
    ) -> Option<VertexRowStats> {
        None
    }
}

fn bedrock_input(usage: Value) -> BatchOutputUsageInput {
    input(json!({
        "kind": "output_usage",
        "provider": "bedrock",
        "model_name": "bedrock-batch",
        "model_info": batch_model_info(),
        "content": {"kind": "lines", "lines": [{
            "custom_id": "b",
            "modelOutput": {"model": "bedrock-batch", "usage": usage},
        }]},
    }))
}

const fn prompt_only_usage(prompt_tokens: i64) -> SummedUsage {
    SummedUsage {
        prompt_tokens,
        completion_tokens: 0,
        total_tokens: prompt_tokens,
        prompt_tokens_details: None,
        completion_tokens_details: None,
        numeric_extras: BTreeMap::new(),
    }
}

#[test]
fn bedrock_prefers_embedding_usage_over_the_usage_object() {
    let providers = EmbeddingProviders {
        embedding: Some(prompt_only_usage(7)),
        converse: ConverseUsageOutcome::Usage(Box::new(prompt_only_usage(99))),
    };
    let result = run(
        &bedrock_input(json!({"inputTextTokenCount": 5})),
        &providers,
    )
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
        &bedrock_input(json!({"inputTokens": 3, "outputTokens": 2})),
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
        &bedrock_input(json!({"input_tokens": 4, "output_tokens": 1})),
        &providers,
    )
    .expect("the batch is costed");
    assert_eq!(result.usage, totals(4, 1, 5));
}

#[test]
fn bedrock_success_needs_model_output_and_no_error() {
    let decoded = input(json!({
        "kind": "output_usage",
        "provider": "bedrock",
        "model_name": "bedrock-batch",
        "model_info": batch_model_info(),
        "content": {"kind": "lines", "lines": [
            {"custom_id": "ok", "modelOutput": {"usage": {"input_tokens": 4, "output_tokens": 1}}},
            {"custom_id": "errored", "error": {"message": "boom"}, "modelOutput": {
                "usage": {"input_tokens": 9, "output_tokens": 9},
            }},
            {"custom_id": "empty"},
        ]},
    }));
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
        run(&bedrock_input(json!({"inputTokens": "x"})), &providers).expect("the batch is costed");
    assert_eq!(result.cost, Decimal::ZERO);
    assert_eq!(result.successful_requests, 1);
    assert!(result.diagnostics.is_empty());
}

#[derive(Clone, Default)]
struct CapturedLogs(Arc<Mutex<Vec<(Level, String, String)>>>);

impl CapturedLogs {
    fn take(&self) -> Vec<(Level, String, String)> {
        match self.0.lock() {
            Ok(mut entries) => mem::take(&mut entries),
            Err(poisoned) => mem::take(&mut poisoned.into_inner()),
        }
    }
}

#[derive(Default)]
struct CapturedMessage {
    message: String,
    custom_id: String,
}

impl Visit for CapturedMessage {
    fn record_str(&mut self, field: &Field, value: &str) {
        match field.name() {
            "message" => value.clone_into(&mut self.message),
            "custom_id" => value.clone_into(&mut self.custom_id),
            _ => {}
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        match field.name() {
            "message" => self.message = format!("{value:?}"),
            "custom_id" => self.custom_id = format!("{value:?}"),
            _ => {}
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for CapturedLogs {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let mut fields = CapturedMessage::default();
        event.record(&mut fields);
        self.0.lock().unwrap_or_else(PoisonError::into_inner).push((
            *event.metadata().level(),
            fields.message,
            fields.custom_id,
        ));
    }
}

fn run_captured(
    input: &BatchOutputUsageInput,
    providers: &dyn BatchOutputProviders,
) -> (
    Result<BatchOutputUsageResult, BatchOutputUsageError>,
    Vec<(Level, String, String)>,
) {
    let logs = CapturedLogs::default();
    let subscriber = tracing_subscriber::registry().with(logs.clone());
    let result = with_default(subscriber, || run(input, providers));
    (result, logs.take())
}

fn logged(logs: &[(Level, String, String)], level: Level, message: &str) -> bool {
    logs.iter()
        .any(|(entry_level, entry, _custom_id)| *entry_level == level && entry.starts_with(message))
}

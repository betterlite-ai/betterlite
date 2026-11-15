use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;
use std::mem;
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

use crate::api::api_boundaries::ApiLimits;
use crate::api::batch_output_usage::pricing::BatchRowPricer;
use crate::api::batch_output_usage::tokens::BatchTokenCounter;
use crate::api::batch_output_usage::usage::response_model;
use crate::api::batch_output_usage::{
    BatchOutputProviders, BatchOutputUsage, BatchOutputUsageError, BatchOutputUsageInput,
    BatchOutputUsageResult, BatchUsageTotals, ConverseUsageOutcome, VertexRowStats,
};

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

pub(super) struct NoProviders;

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

pub(super) struct ByModelLookup {
    pub(super) response: PriceRecord,
    pub(super) deployment: PriceRecord,
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

pub(super) fn run(
    input: &BatchOutputUsageInput,
    providers: &dyn BatchOutputProviders,
) -> Result<BatchOutputUsageResult, BatchOutputUsageError> {
    run_with_lookup(input, &NoLookup, providers)
}

pub(super) fn run_with_lookup(
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

pub(super) fn input(value: Value) -> BatchOutputUsageInput {
    serde_json::from_value(value).expect("the fixture input decodes")
}

pub(super) fn json_entry(value: Value) -> Json {
    serde_json::from_value(value).expect("the batch entry decodes")
}

pub(super) fn batch_model_info() -> ModelInfo {
    serde_json::from_str(
        r#"{"input_cost_per_token_batches":0.5,"output_cost_per_token_batches":0.25}"#,
    )
    .expect("the batch rates decode")
}

pub(super) fn ocr_model_info() -> ModelInfo {
    serde_json::from_str(r#"{"ocr_cost_per_page_batches":0.5}"#).expect("the OCR rates decode")
}

pub(super) const fn totals(
    prompt_tokens: i64,
    completion_tokens: i64,
    total_tokens: i64,
) -> BatchUsageTotals {
    BatchUsageTotals {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        reasoning_tokens: 0,
        cache_read_tokens: 0,
        cache_creation_tokens: 0,
    }
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
        || response_model(Some(response)),
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

pub(super) struct FakeVertex;

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

#[derive(Debug, PartialEq, Eq)]
pub(super) struct RecordedCall {
    pub(super) model: String,
    pub(super) text: Option<String>,
    pub(super) messages: Option<usize>,
    pub(super) count_response_tokens: Option<bool>,
}

#[derive(Default)]
pub(super) struct ScriptedCounter {
    answer: usize,
    calls: Mutex<Vec<RecordedCall>>,
}

impl ScriptedCounter {
    pub(super) const fn with_answer(answer: usize) -> Self {
        Self {
            answer,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn taken(&self) -> Vec<RecordedCall> {
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

pub(super) struct EmbeddingProviders {
    pub(super) embedding: Option<SummedUsage>,
    pub(super) converse: ConverseUsageOutcome,
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

pub(super) fn bedrock_input(usage: Value) -> BatchOutputUsageInput {
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

pub(super) const fn prompt_only_usage(prompt_tokens: i64) -> SummedUsage {
    SummedUsage {
        prompt_tokens,
        completion_tokens: 0,
        total_tokens: prompt_tokens,
        prompt_tokens_details: None,
        completion_tokens_details: None,
        numeric_extras: BTreeMap::new(),
    }
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

pub(super) fn run_captured(
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

pub(super) fn logged(logs: &[(Level, String, String)], level: Level, message: &str) -> bool {
    logs.iter()
        .any(|(entry_level, entry, _custom_id)| *entry_level == level && entry.starts_with(message))
}

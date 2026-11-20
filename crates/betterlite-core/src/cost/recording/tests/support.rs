use std::collections::BTreeMap;
use std::fmt::Debug;
use std::future::Future;
use std::mem;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_logging::fields::usage::Usage;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_models::resolution::ProviderIdentity;
use betterlite_models::tiered_pricing::Scalar;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde_json::Number;
use tokio::runtime::Builder;
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context as LayerContext, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use super::super::view;
use super::super::{
    CostCalculation, CostRecorder, CostRequest, GenerateContentConversion, MantleRegionResolver,
    PortFailure, RecordInput, RecordResponse, VertexRegionNormalizer,
};
use crate::cost::context::ProviderInference;
use crate::cost::prepare::{ResponseView, TypedResponse};
use crate::cost::usage::{BillingUsage, ResponseKind, get_usage_object};

pub(super) const INIT: &str = r#"{"model":"gpt-4o","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1"}"#;
pub(super) const MANAGEMENT: &str = r#"{"model":"gpt-4o","call_type":"get_responses","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1"}"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("boom")]
pub(super) struct TestFailure;

#[derive(Debug, Default, PartialEq)]
pub(super) struct Observed {
    pub(super) model: String,
    pub(super) cache_hit: Option<bool>,
    pub(super) provider: Option<String>,
    pub(super) base_model: Option<String>,
    pub(super) call_type: String,
    pub(super) custom_pricing: bool,
    pub(super) prompt: String,
    pub(super) router_model_id: Option<String>,
    pub(super) data_residency: Option<String>,
    pub(super) vertex_location: Option<String>,
    pub(super) region_name: Option<String>,
    pub(super) usage: Option<(i64, i64)>,
}

pub(super) struct FakeCalculator {
    result: Result<Decimal, TestFailure>,
    observed: Mutex<Vec<Observed>>,
}

impl Default for FakeCalculator {
    fn default() -> Self {
        Self {
            result: Ok(Decimal::ZERO),
            observed: Mutex::new(Vec::new()),
        }
    }
}

impl FakeCalculator {
    pub(super) fn observe(&self) -> Vec<Observed> {
        match self.observed.lock() {
            Ok(mut found) => mem::take(&mut *found),
            Err(poisoned) => mem::take(&mut poisoned.into_inner()),
        }
    }
}

#[async_trait::async_trait]
impl CostCalculation for FakeCalculator {
    async fn calculate(&self, request: &CostRequest<'_>) -> Result<Decimal, PortFailure> {
        let usage = usage_counts(request.response);
        {
            let mut observed = match self.observed.lock() {
                Ok(found) => found,
                Err(poisoned) => poisoned.into_inner(),
            };
            observed.push(Observed {
                model: request.model.to_owned(),
                cache_hit: request.cache_hit,
                provider: request.custom_llm_provider.map(str::to_owned),
                base_model: request.base_model.map(str::to_owned),
                call_type: request.call_type.to_owned(),
                custom_pricing: request.custom_pricing,
                prompt: request.prompt.to_owned(),
                router_model_id: request.router_model_id.map(str::to_owned),
                data_residency: request.data_residency.map(str::to_owned),
                vertex_location: request.vertex_location.map(str::to_owned),
                region_name: request.region_name.map(str::to_owned),
                usage,
            });
        }
        match self.result {
            Ok(cost) => Ok(cost),
            Err(error) => Err(Box::new(error)),
        }
    }
}

#[derive(Default)]
pub(super) struct FakeCatalog {
    entries: BTreeMap<String, PriceRecord>,
}

impl FakeCatalog {
    pub(super) fn with_entry(model: &str, fields: &[(&str, Scalar)]) -> Self {
        let mut record = PriceRecord::new();
        for (name, value) in fields {
            record.insert((*name).to_owned(), value.clone());
        }
        record.insert("key".to_owned(), Scalar::Text(model.to_owned()));
        let mut entries = BTreeMap::new();
        entries.insert(model.to_owned(), record);
        Self { entries }
    }

    pub(super) fn with_unkeyed_entry(model: &str, name: &str, value: Scalar) -> Self {
        let mut record = PriceRecord::new();
        record.insert(name.to_owned(), value);
        let mut entries = BTreeMap::new();
        entries.insert(model.to_owned(), record);
        Self { entries }
    }
}

impl PriceLookup for FakeCatalog {
    fn effective_model_info(
        &self,
        model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        self.entries.get(model).cloned()
    }

    fn raw_cost_map_entry(&self, key: &str) -> Option<PriceRecord> {
        self.entries.get(key).cloned()
    }

    fn effective_priced_model(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<PricedModel> {
        self.effective_model_info(model, custom_llm_provider)
            .map(|record| PricedModel {
                record,
                tiered_pricing: Vec::new(),
                off_peak_pricing: None,
                off_peak_cache_read_present: false,
            })
    }
}

pub(super) struct FakeInference;

impl ProviderInference for FakeInference {
    type Failure = TestFailure;

    fn inferred_provider(&self, _model: &str) -> Result<ProviderIdentity, TestFailure> {
        Ok(ProviderIdentity::Custom("test".to_owned()))
    }
}

#[derive(Default)]
pub(super) struct RecordingRegions {
    seen: Mutex<Vec<Option<String>>>,
}

impl RecordingRegions {
    pub(super) fn configured(&self) -> Vec<Option<String>> {
        match self.seen.lock() {
            Ok(mut found) => mem::take(&mut *found),
            Err(poisoned) => mem::take(&mut poisoned.into_inner()),
        }
    }
}

impl VertexRegionNormalizer for RecordingRegions {
    fn region(&self, _model: &str, configured: Option<&str>) -> String {
        let seen = configured.unwrap_or("none");
        match self.seen.lock() {
            Ok(mut found) => found.push(configured.map(str::to_owned)),
            Err(poisoned) => poisoned.into_inner().push(configured.map(str::to_owned)),
        }
        format!("normalized-{seen}")
    }
}

pub(super) struct RecordingMantle {
    region: String,
    seen: Mutex<Vec<IndexMap<String, Json>>>,
}

impl Default for RecordingMantle {
    fn default() -> Self {
        Self {
            region: "us-east-1".to_owned(),
            seen: Mutex::new(Vec::new()),
        }
    }
}

impl RecordingMantle {
    pub(super) fn calls(&self) -> Vec<IndexMap<String, Json>> {
        match self.seen.lock() {
            Ok(mut found) => mem::take(&mut *found),
            Err(poisoned) => mem::take(&mut poisoned.into_inner()),
        }
    }
}

impl MantleRegionResolver for RecordingMantle {
    fn region(&self, litellm_params: &IndexMap<String, Json>) -> Result<String, PortFailure> {
        match self.seen.lock() {
            Ok(mut found) => found.push(litellm_params.clone()),
            Err(poisoned) => poisoned.into_inner().push(litellm_params.clone()),
        }
        Ok(self.region.clone())
    }
}

#[derive(Default)]
pub(super) struct FakeConversion {
    converted: Option<Json>,
}

impl GenerateContentConversion for FakeConversion {
    fn convert(&self, _body: &Json, _model: &str) -> Result<Json, PortFailure> {
        match self.converted.clone() {
            Some(converted) => Ok(converted),
            None => Err(Box::new(TestFailure)),
        }
    }
}

pub(super) struct Fakes {
    calculator: FakeCalculator,
    catalog: FakeCatalog,
    inference: FakeInference,
    conversion: FakeConversion,
    regions: RecordingRegions,
    mantle: RecordingMantle,
    vertex: VertexSettings,
    ptu: bool,
}

impl Default for Fakes {
    fn default() -> Self {
        Self {
            calculator: FakeCalculator::default(),
            catalog: FakeCatalog::default(),
            inference: FakeInference,
            conversion: FakeConversion::default(),
            regions: RecordingRegions::default(),
            mantle: RecordingMantle::default(),
            vertex: VertexSettings::default(),
            ptu: false,
        }
    }
}

impl Fakes {
    pub(super) fn with_cost(mut self, cost: Decimal) -> Self {
        self.calculator.result = Ok(cost);
        self
    }

    pub(super) fn with_failure(mut self) -> Self {
        self.calculator.result = Err(TestFailure);
        self
    }

    pub(super) fn with_catalog(mut self, catalog: FakeCatalog) -> Self {
        self.catalog = catalog;
        self
    }

    pub(super) fn with_ptu(mut self) -> Self {
        self.ptu = true;
        self
    }

    pub(super) fn with_conversion(mut self, converted: Option<Json>) -> Self {
        self.conversion.converted = converted;
        self
    }

    pub(super) fn with_vertex_location(mut self, location: Option<&str>) -> Self {
        self.vertex.location = location.map(str::to_owned);
        self
    }

    pub(super) fn recorder(&self) -> CostRecorder<'_, FakeInference> {
        CostRecorder {
            calculator: &self.calculator,
            lookup: &self.catalog,
            inference: &self.inference,
            conversion: &self.conversion,
            regions: &self.regions,
            mantle: &self.mantle,
            vertex: &self.vertex,
            ptu_cost_attribution: self.ptu,
        }
    }

    pub(super) fn observe(&self) -> Vec<Observed> {
        self.calculator.observe()
    }

    pub(super) fn regions(&self) -> Vec<Option<String>> {
        self.regions.configured()
    }

    pub(super) fn mantle_calls(&self) -> Vec<IndexMap<String, Json>> {
        self.mantle.calls()
    }
}

#[derive(Clone, Default)]
pub(super) struct CapturedLogs(Arc<Mutex<Vec<String>>>);

impl CapturedLogs {
    fn take(&self) -> Vec<String> {
        match self.0.lock() {
            Ok(mut messages) => mem::take(&mut messages),
            Err(poisoned) => mem::take(&mut poisoned.into_inner()),
        }
    }
}

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            value.clone_into(&mut self.0);
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for CapturedLogs {
    fn on_event(&self, event: &Event<'_>, _context: LayerContext<'_, Inner>) {
        if *event.metadata().level() != Level::WARN {
            return;
        }
        let mut message = Message::default();
        event.record(&mut message);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(message.0);
    }
}

pub(super) fn capture<T, F>(call: F) -> (T, Vec<String>)
where
    F: FnOnce() -> T,
{
    let logs = CapturedLogs::default();
    let subscriber = registry().with(logs.clone());
    let value = with_default(subscriber, call);
    (value, logs.take())
}

pub(super) fn run<T, F>(future: F) -> T
where
    F: Future<Output = T>,
{
    match Builder::new_current_thread().build() {
        Ok(runtime) => runtime.block_on(future),
        Err(error) => panic!("current-thread runtime: {error}"),
    }
}

pub(super) fn json_text(text: &str) -> Json {
    match serde_json::from_str::<Json>(text) {
        Ok(value) => value,
        Err(error) => panic!("invalid test json: {error}"),
    }
}

pub(super) fn object_fields(text: &str) -> IndexMap<String, Json> {
    match json_text(text) {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("test json is not an object")
        }
    }
}

pub(super) fn decimal(text: &str) -> Decimal {
    Decimal::from_str_exact(text).unwrap()
}

pub(super) fn a_price(amount: i64) -> Scalar {
    Scalar::Number(Number::from(amount))
}

pub(super) fn chat_usage() -> Usage {
    Usage {
        prompt_tokens: 10,
        completion_tokens: 5,
        total_tokens: 15,
        prompt_tokens_details: None,
        completion_tokens_details: None,
        server_tool_use: None,
        cost: None,
        extras: IndexMap::new(),
    }
}

pub(super) fn typed(response: TypedResponse<'_>) -> RecordResponse<'_> {
    RecordResponse::Typed {
        response,
        background: false,
    }
}

pub(super) fn named<'usage>(
    usage: &'usage Usage,
    hidden: &'usage IndexMap<String, Json>,
    model: &'usage str,
) -> TypedResponse<'usage> {
    TypedResponse {
        kind: ResponseKind::Chat,
        model: Some(model),
        usage: BillingUsage::Chat(usage),
        service_tier: None,
        hidden_params: Some(hidden),
    }
}

pub(super) fn typed_response<'usage>(
    usage: &'usage Usage,
    hidden: &'usage IndexMap<String, Json>,
) -> TypedResponse<'usage> {
    named(usage, hidden, "gpt-4o")
}

pub(super) fn state(init: &str) -> LoggingState {
    let parsed = match serde_json::from_str::<LoggingInit>(init) {
        Ok(parsed) => parsed,
        Err(error) => panic!("invalid init: {error}"),
    };
    let redactor = match Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH) {
        Ok(found) => found,
        Err(error) => panic!("redactor: {error}"),
    };
    match LoggingState::new(
        parsed,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || String::from("trace"),
        Duration::default(),
    ) {
        Ok(state) => state,
        Err(error) => panic!("logging state: {error}"),
    }
}

pub(super) fn openai(log: &mut LoggingState) {
    provider(log, "openai");
}

pub(super) fn provider(log: &mut LoggingState, name: &str) {
    log.model_call_details.insert(
        "custom_llm_provider".to_owned(),
        Json::Text(name.to_owned()),
    );
}

fn usage_counts(response: &ResponseView<'_>) -> Option<(i64, i64)> {
    let billing = view::billing_response(response);
    match get_usage_object(&billing) {
        Ok(Some(found)) => Some((found.prompt_tokens, found.completion_tokens)),
        Ok(None) | Err(_) => None,
    }
}

pub(super) fn stored_diagnostic(log: &LoggingState) -> Option<Json> {
    log.model_call_details.get("zero_cost_diagnostic").cloned()
}

pub(super) fn stored_failure(log: &LoggingState) -> Option<Json> {
    log.model_call_details
        .get("response_cost_failure_debug_information")
        .cloned()
}

pub(super) fn earlier_finding() -> Json {
    json_text(
        r#"{"reason":"pricing_not_applied","pricing_model":"older","missing_pricing_keys":[]}"#,
    )
}

pub(super) fn priced_gpt4o(fakes: Fakes) -> Fakes {
    fakes.with_catalog(FakeCatalog::with_entry(
        "gpt-4o",
        &[
            ("input_cost_per_token", a_price(1)),
            ("output_cost_per_token", a_price(1)),
        ],
    ))
}

pub(super) fn record(
    recorder: &CostRecorder<'_, FakeInference>,
    log: &mut LoggingState,
    response: RecordResponse<'_>,
) -> Option<Decimal> {
    respond(recorder, log, response, None)
}

pub(super) fn respond(
    recorder: &CostRecorder<'_, FakeInference>,
    log: &mut LoggingState,
    response: RecordResponse<'_>,
    cache_hit: Option<bool>,
) -> Option<Decimal> {
    run(recorder.record(
        log,
        RecordInput {
            response,
            cache_hit,
            litellm_model_name: None,
            router_model_id: None,
        },
    ))
}

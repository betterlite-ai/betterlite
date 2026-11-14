#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;
use std::str::FromStr as _;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use betterlite_config::cost_adjustments::AdjustmentEntry;
use betterlite_logging::cost_breakdown::{CostBreakdownInput, CostBreakdownTarget};
use betterlite_logging::fields::usage::Usage;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::cost_usage::tool_projection::ToolUsage;
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::provider_model_sets::ProviderModelSets;
use betterlite_models::resolution::{ProviderIdentity, ResolutionContext, ResolvedModel};
use betterlite_models::tiered_pricing::Scalar;
use betterlite_models::tool_fees::billing::{BuiltInToolBilling, BuiltInToolParams};
use betterlite_models::tool_fees::error::LookupFailed;
use betterlite_models::tool_fees::model_info::{Rate, SearchContextRates, ToolModelInfo};
use betterlite_models::tool_fees::ports::{ToolPricePolicy, ToolPriceSource};
use betterlite_models::tool_fees::prices::ToolPrices;
use betterlite_models::tool_signals::response::{OtherView, ToolBillingResponse};
use betterlite_tokenizer::count::error::CountError;
use indexmap::IndexMap;
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;
use serde_json::Number;

use crate::cost::call_price::policy::ProviderPricePolicy;
use crate::cost::completion::args::{
    ChargeInputs, CompletionArgs, CostCalculator, ResponseParts, ToolInputs,
};
use crate::cost::completion::completion_cost;
use crate::cost::completion::error::CompletionCostError;
use crate::cost::context::ProviderInference;
use crate::cost::image_dispatch::options::SizePattern;
use crate::cost::image_dispatch::policy::{ImagePricePolicy, ImageResponseView, ImageRoute};
use crate::cost::prepare::{
    BillingTokenCounter, CompletionBillingInput, DictionaryResponse, ResponseView, TypedResponse,
};
use crate::cost::response::response_cost_calculator;
use crate::cost::special::policy::{ModelResolver, SpecialPricePolicy};
use crate::cost::special::replicate::ReplicateStamps;
use crate::cost::special::request::{
    ImageParts, LoggingView, SpecialServices, TranscriptionDuration,
};
use crate::cost::usage::{BillingUsage, ResponseKind};

pub fn dec(text: &str) -> Decimal {
    Decimal::from_str(text).expect("the fixture is a decimal")
}

pub fn json(document: &str) -> Json {
    serde_json::from_str(document).expect("the fixture is JSON")
}

pub fn map(document: &str) -> IndexMap<String, Json> {
    let Json::Object(found) = json(document) else {
        panic!("the fixture is an object");
    };
    found
}

pub fn number(text: &str) -> Json {
    Json::Number(Number::from_str(text).expect("the fixture is a number"))
}

pub fn chat_usage(prompt: i64, completion: i64) -> Usage {
    Usage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        total_tokens: prompt + completion,
        prompt_tokens_details: None,
        completion_tokens_details: None,
        server_tool_use: None,
        cost: None,
        extras: IndexMap::new(),
    }
}

pub fn chat_view<'view>(
    usage: &'view Usage,
    model: Option<&'view str>,
    hidden_params: Option<&'view IndexMap<String, Json>>,
) -> ResponseView<'view> {
    ResponseView::Typed(TypedResponse {
        kind: ResponseKind::Chat,
        model,
        usage: BillingUsage::Chat(usage),
        service_tier: None,
        hidden_params,
    })
}

pub fn dictionary_view(usage: &Usage) -> ResponseView<'_> {
    ResponseView::Dictionary(DictionaryResponse {
        model: Some("gpt-4o"),
        usage: BillingUsage::Chat(usage),
        service_tier: None,
    })
}

pub fn priced(fields: &[(&str, &str)]) -> PriceRecord {
    fields
        .iter()
        .map(|(key, value)| {
            let found = Number::from_str(value).expect("the fixture is a number");
            ((*key).to_owned(), Scalar::Number(found))
        })
        .collect()
}

struct Catalog(BTreeMap<String, PriceRecord>);

impl PriceLookup for Catalog {
    fn effective_model_info(&self, model: &str, _provider: Option<&str>) -> Option<PriceRecord> {
        self.0.get(model).cloned()
    }

    fn raw_cost_map_entry(&self, key: &str) -> Option<PriceRecord> {
        self.0.get(key).cloned()
    }

    fn effective_priced_model(&self, model: &str, _provider: Option<&str>) -> Option<PricedModel> {
        self.0.get(model).map(|entry| PricedModel {
            record: entry.clone(),
            tiered_pricing: Vec::new(),
            off_peak_pricing: None,
            off_peak_cache_read_present: false,
        })
    }
}

struct Resolver;

impl ModelResolver for Resolver {
    type Failure = &'static str;

    fn resolve_model(&self, _model: &str) -> Result<ResolvedModel, &'static str> {
        Err("the rig resolves no model")
    }
}

struct Images;

impl ImagePricePolicy for Images {
    type Error = Infallible;

    fn image_cost(&self, _route: &ImageRoute<'_>) -> Result<Decimal, Infallible> {
        Ok(Decimal::ZERO)
    }
}

// No provider formula is installed: every policy operation answers with its unavailable adapter.
struct Silent;

impl SpecialPricePolicy for Silent {}

impl ProviderPricePolicy for Silent {}

struct Inference;

impl ProviderInference for Inference {
    type Failure = Infallible;

    fn inferred_provider(&self, _model: &str) -> Result<ProviderIdentity, Infallible> {
        Ok(ProviderIdentity::Custom("openai".to_owned()))
    }
}

struct Counter;

#[async_trait]
impl BillingTokenCounter for Counter {
    async fn count_messages(&self, _model: &str, _messages: &[Json]) -> Result<usize, CountError> {
        Ok(3)
    }

    async fn count_text(&self, _model: &str, text: &str) -> Result<usize, CountError> {
        Ok(text.chars().count())
    }
}

// A search-priced model: the medium context rate is one cent per call.
struct Tools(Option<ToolModelInfo>);

impl ToolPriceSource for Tools {
    fn model_info(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Result<ToolModelInfo, LookupFailed> {
        self.0.clone().ok_or(LookupFailed)
    }

    fn container_session_rate(&self, _provider: &str) -> Result<Rate, LookupFailed> {
        Err(LookupFailed)
    }
}

impl ToolPricePolicy for Tools {
    fn web_search_cost(
        &self,
        _provider: &str,
        _usage: &ToolUsage,
        _model_info: &ToolModelInfo,
    ) -> Option<Decimal> {
        None
    }

    fn google_maps_cost(
        &self,
        _provider: &str,
        _requests: i64,
        _model_info: &ToolModelInfo,
    ) -> Option<Decimal> {
        None
    }
}

// The amounts a target received; the input type itself is neither Clone nor Debug.
#[derive(Debug, PartialEq, Eq)]
pub struct Recorded {
    pub input_cost: Decimal,
    pub output_cost: Decimal,
    pub total_cost: Decimal,
    pub tool_usage_cost: Decimal,
    pub additional_costs: Option<BTreeMap<String, Decimal>>,
    pub original_cost: Option<Decimal>,
    pub discount_percent: Option<Decimal>,
    pub discount_amount: Option<Decimal>,
    pub margin_percent: Option<Decimal>,
    pub margin_fixed_amount: Option<Decimal>,
    pub margin_total_amount: Option<Decimal>,
    pub token_costs: [Option<Decimal>; 3],
}

#[derive(Default)]
pub struct Stored(pub Option<Recorded>);

impl CostBreakdownTarget for Stored {
    type Failure = Infallible;

    fn set_cost_breakdown(&mut self, input: &CostBreakdownInput) -> Result<(), Infallible> {
        self.0 = Some(Recorded {
            input_cost: input.input_cost,
            output_cost: input.output_cost,
            total_cost: input.total_cost,
            tool_usage_cost: input.tool_usage_cost,
            additional_costs: input.additional_costs.clone(),
            original_cost: input.original_cost,
            discount_percent: input.discount_percent,
            discount_amount: input.discount_amount,
            margin_percent: input.margin_percent,
            margin_fixed_amount: input.margin_fixed_amount,
            margin_total_amount: input.margin_total_amount,
            token_costs: [
                input.cache_read_cost,
                input.cache_creation_cost,
                input.reasoning_cost,
            ],
        });
        Ok(())
    }
}

#[derive(Debug)]
pub struct Refused;

pub struct Refusing;

impl CostBreakdownTarget for Refusing {
    type Failure = Refused;

    fn set_cost_breakdown(&mut self, _input: &CostBreakdownInput) -> Result<(), Refused> {
        Err(Refused)
    }
}

pub struct Rig {
    catalog: Catalog,
    tools: Tools,
    pattern: SizePattern,
    database: TimeZoneDatabase,
    models: ProviderModelSets,
    pub discount: BTreeMap<String, AdjustmentEntry>,
    pub margin: BTreeMap<String, AdjustmentEntry>,
    catalog_model: ModelCatalog,
    names: BTreeSet<String>,
    environment: BTreeMap<String, String>,
}

pub fn rig(prices: &[(&str, &[(&str, &str)])]) -> Rig {
    Rig {
        catalog: Catalog(
            prices
                .iter()
                .map(|(key, fields)| ((*key).to_owned(), priced(fields)))
                .collect(),
        ),
        tools: Tools(None),
        pattern: SizePattern::new().expect("the size pattern compiles"),
        database: TimeZoneDatabase::from_env(),
        models: ProviderModelSets::default(),
        discount: BTreeMap::new(),
        margin: BTreeMap::new(),
        catalog_model: ModelCatalog::default(),
        names: BTreeSet::new(),
        environment: BTreeMap::new(),
    }
}

// One cent per web search call at the medium context size.
pub fn searching() -> ToolModelInfo {
    ToolModelInfo {
        litellm_provider: Some("openai".to_owned()),
        search_context: SearchContextRates {
            low: Rate::Absent,
            medium: Rate::Value(dec("0.01")),
            high: Rate::Absent,
        },
        file_search_per_gb_per_day: Rate::Absent,
        file_search_per_1k_calls: Rate::Absent,
        vector_store_per_gb_per_day: Rate::Absent,
        computer_input_per_1k: Rate::Absent,
        computer_output_per_1k: Rate::Absent,
        code_interpreter_per_session: Rate::Absent,
    }
}

impl Rig {
    pub fn with_tool_prices(mut self, info: ToolModelInfo) -> Self {
        self.tools = Tools(Some(info));
        self
    }

    fn context(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.catalog_model,
            known_model_names: &self.names,
            custom_providers: &self.names,
            environment: &self.environment,
            registry: JSONProviderRegistry::default(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }

    fn services<'rig>(
        &'rig self,
        context: &'rig ResolutionContext<'rig>,
    ) -> CostCalculator<'rig, Resolver, Images, Inference> {
        CostCalculator {
            special: SpecialServices {
                lookup: &self.catalog,
                resolver: &Resolver,
                image_policy: &Images,
                special_policy: &Silent,
                size_pattern: &self.pattern,
                database: &self.database,
                replicate_models: &self.models,
                replicate_price: dec("0.0014"),
                discount: &self.discount,
                margin: &self.margin,
            },
            catalog: &self.catalog,
            inference: &Inference,
            counter: &Counter,
            price_policy: &Silent,
            resolution: context,
            tools: BuiltInToolBilling {
                source: &self.tools,
                policy: &self.tools,
                prices: &ToolPrices::PINNED,
            },
        }
    }

    pub async fn charge<Target>(
        &self,
        args: &CompletionArgs<'_>,
        target: Option<&mut Target>,
    ) -> Result<Decimal, CompletionCostError<Infallible>>
    where
        Target: CostBreakdownTarget,
    {
        let context = self.context();
        completion_cost(&self.services(&context), args, &mut clock, target).await
    }

    pub async fn respond<Target>(
        &self,
        args: &CompletionArgs<'_>,
        cache_hit: Option<bool>,
        target: Option<&mut Target>,
    ) -> Result<Decimal, CompletionCostError<Infallible>>
    where
        Target: CostBreakdownTarget,
    {
        let context = self.context();
        response_cost_calculator(
            &self.services(&context),
            args,
            cache_hit,
            &mut clock,
            target,
        )
        .await
    }
}

fn clock() -> SystemTime {
    UNIX_EPOCH
}

fn charge_inputs<'inputs>() -> ChargeInputs<'inputs> {
    ChargeInputs {
        passthrough_image: false,
        total_time: None,
        data_residency: None,
        vertex_location: None,
        custom_cost_per_token: None,
        custom_cost_per_second: None,
        logging: LoggingView {
            model: None,
            params: None,
            timing: None,
        },
        response: ResponseParts {
            cost_response: None,
            image: ImageParts {
                response: ImageResponseView::Other,
                quality: None,
                image_count: None,
                size: None,
            },
            video_usage: None,
            transcription: TranscriptionDuration::default(),
            rerank: None,
            protocol: None,
            replicate: ReplicateStamps::default(),
            model_response: None,
        },
        ocr_billing: None,
        vector_response: None,
        search_hidden_params: None,
        server_tool_projection: None,
        tools: ToolInputs {
            response: ToolBillingResponse::Other(OtherView { usage: None }),
            params: BuiltInToolParams::default(),
            google_maps_requests: None,
        },
    }
}

pub fn args<'args>(response: Option<&'args ResponseView<'args>>) -> CompletionArgs<'args> {
    CompletionArgs {
        billing: CompletionBillingInput {
            model: Some("gpt-4o"),
            prompt: "",
            messages: &[],
            completion: "",
            call_type: None,
            custom_llm_provider: Some("openai"),
            region_name: None,
            router_model_id: None,
            custom_pricing: None,
            base_model: None,
            optional_params: None,
            service_tier: None,
            response,
        },
        charge: charge_inputs(),
    }
}

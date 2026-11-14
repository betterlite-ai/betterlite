use std::collections::{BTreeMap, BTreeSet};
use std::time::UNIX_EPOCH;

use async_trait::async_trait;
use betterlite_bedrock::mantle_inputs::region::split_region_prefix;
use betterlite_context::bedrock_model::{get_bedrock_base_model, strip_bedrock_routing_prefix};
use betterlite_core::cost::call_price::CustomCostPerToken;
use betterlite_core::cost::call_price::policy::error::PolicyError;
use betterlite_core::cost::completion::args::{
    ChargeInputs, CompletionArgs, CostCalculator, ResponseParts, ToolInputs,
};
use betterlite_core::cost::completion::completion_cost;
use betterlite_core::cost::completion::lookup::{CatalogLookup, LookupSettings};
use betterlite_core::cost::image_dispatch::options::SizePattern;
use betterlite_core::cost::image_dispatch::policy::ImageResponseView;
use betterlite_core::cost::prepare::{
    BillingTokenCounter, CompletionBillingInput, ResponseView, TypedResponse,
};
use betterlite_core::cost::realtime::RealtimeBilling;
use betterlite_core::cost::response::response_cost_calculator;
use betterlite_core::cost::special::policy::{SearchQuery, SpecialPricePolicy};
use betterlite_core::cost::special::replicate::ReplicateStamps;
use betterlite_core::cost::special::request::{
    HiddenDuration, ImageParts, LoggingView, SpecialServices, TranscriptionDuration,
};
use betterlite_core::cost::usage::{BillingUsage, ResponseKind};
use betterlite_logging::cost_breakdown::{CostBreakdownInput, CostBreakdownTarget};
use betterlite_models::decisions::Json;
use betterlite_models::image_cost::ImageBilling;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::model_metadata_resolution::names::NameNormalizers;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::provider_model_sets::ProviderModelSets;
use betterlite_models::token_cost::GenericCostResult;
use betterlite_models::tool_fees::billing::BuiltInToolBilling;
use betterlite_models::tool_fees::prices::ToolPrices;
use betterlite_provider::pricing::ProviderPricing;
use betterlite_provider::pricing::grounding::{GroundingPricing, ProviderFormulas};
use betterlite_provider::pricing::image::ProviderImagePricing;
use betterlite_tokenizer::count::error::CountError;
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde_json::json;

use self::input::{Function, Input, Kind, adjustments, hidden_params, usage_block};

mod input;

struct Counter;

#[async_trait]
impl BillingTokenCounter for Counter {
    async fn count_messages(&self, _model: &str, _messages: &[Json]) -> Result<usize, CountError> {
        Ok(0)
    }

    async fn count_text(&self, _model: &str, _text: &str) -> Result<usize, CountError> {
        Ok(0)
    }
}

// The lookup adapter the token stage asks for; a case only prices a search it holds a rate for.
struct SearchPricing<'rates> {
    rates: &'rates BTreeMap<String, Decimal>,
}

impl SpecialPricePolicy for SearchPricing<'_> {
    fn search_cost(&self, query: &SearchQuery<'_>) -> Result<GenericCostResult, PolicyError> {
        let Some(rate) = self.rates.get(query.model) else {
            return Err(PolicyError::UnavailableAdapter(
                query.provider.unwrap_or("search").to_owned(),
            ));
        };
        let queries = Decimal::from(query.number_of_queries);
        let Some(prompt_cost) = rate.checked_mul(queries) else {
            return Err(PolicyError::UnavailableAdapter(
                query.provider.unwrap_or("search").to_owned(),
            ));
        };
        Ok(GenericCostResult {
            prompt_cost,
            completion_cost: Decimal::ZERO,
        })
    }
}

// The isolated breakdown receiver: a refusing one proves a setter failure never replaces the charge.
struct Breakdown {
    refuse: bool,
}

impl CostBreakdownTarget for Breakdown {
    type Failure = &'static str;

    fn set_cost_breakdown(&mut self, _input: &CostBreakdownInput) -> Result<(), &'static str> {
        if self.refuse {
            return Err("the receiver refuses the breakdown");
        }
        Ok(())
    }
}

fn failure(class: &str, message: String) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

async fn charge(
    input: &Input,
    target: Option<&mut Breakdown>,
) -> Result<Decimal, Box<crate::helpers::HelperError>> {
    let catalog_bytes = serde_json::to_vec(&input.catalog)
        .map_err(|error| failure("CatalogError", error.to_string()))?;
    let lookup = CatalogLookup::new(
        &catalog_bytes,
        LookupSettings {
            known_model_names: BTreeSet::new(),
            custom_providers: BTreeSet::new(),
            environment: BTreeMap::new(),
            registry: JSONProviderRegistry::default(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
            normalizers: NameNormalizers {
                bedrock_base_model: get_bedrock_base_model,
                bedrock_routing_prefix: strip_bedrock_routing_prefix,
                mantle_region_split: split_region_prefix,
            },
        },
    )
    .map_err(|error| failure("CatalogError", error.to_string()))?;
    let mut clock = || UNIX_EPOCH;
    let (discount, margin) = adjustments(input);
    let database = TimeZoneDatabase::from_env();
    let pattern = SizePattern::new().map_err(|error| failure("PatternError", error.to_string()))?;
    let models = ProviderModelSets::default();
    let image = ProviderImagePricing {
        lookup: &lookup,
        database: &database,
        moment: DateTime::<Utc>::UNIX_EPOCH,
    };
    let tools = GroundingPricing {
        formulas: ProviderFormulas,
        model_key: Some(input.model.as_str()),
    };
    let search_rates = input::search_rates(&input.catalog);
    let search = SearchPricing {
        rates: &search_rates,
    };
    let resolution = lookup.resolution();
    let calculator = CostCalculator {
        special: SpecialServices {
            lookup: &lookup,
            resolver: &lookup,
            image_policy: &image,
            special_policy: &search,
            size_pattern: &pattern,
            database: &database,
            replicate_models: &models,
            replicate_price: Decimal::new(14, 4),
            discount: &discount,
            margin: &margin,
        },
        catalog: &lookup,
        inference: &lookup,
        counter: &Counter,
        price_policy: &ProviderPricing,
        resolution: &resolution,
        tools: BuiltInToolBilling {
            source: &lookup,
            policy: &tools,
            prices: &ToolPrices::PINNED,
        },
    };
    let kind = Kind::of(input);
    let usage = usage_block(&input.usage);
    let hidden = input.header_cost.as_ref().map(hidden_params);
    let transcription_usage = input::transcription_usage(input);
    let transcription = TranscriptionDuration {
        hidden: HiddenDuration::Missing,
        response_duration: input::transcription_duration(input),
    };
    let image_billing = ImageBilling {
        image_count: 1,
        size: None,
        quality: None,
        usage: None,
    };
    let ocr_billing = input::ocr_billing(input);
    let protocol = input::protocol(input);
    let realtime = RealtimeBilling {
        results: input::events(input),
        usage: input::pricing_usage(&input.usage),
        service_tier: None,
    };
    let image_response = if kind == Kind::Image {
        ImageResponseView::Image(&image_billing)
    } else {
        ImageResponseView::Other
    };
    let view = match kind {
        Kind::Chat => ResponseView::Typed(TypedResponse {
            kind: ResponseKind::Chat,
            model: input.response_model.as_deref(),
            usage: BillingUsage::Chat(&usage),
            service_tier: None,
            hidden_params: hidden.as_ref(),
        }),
        Kind::Image => ResponseView::Typed(TypedResponse {
            kind: ResponseKind::Image,
            model: input.response_model.as_deref(),
            usage: BillingUsage::Absent,
            service_tier: None,
            hidden_params: None,
        }),
        Kind::Transcription => ResponseView::Typed(TypedResponse {
            kind: ResponseKind::Transcription,
            model: input.response_model.as_deref(),
            usage: BillingUsage::Transcription(&transcription_usage),
            service_tier: None,
            hidden_params: None,
        }),
        Kind::RealtimeStream => ResponseView::Realtime {
            record: &realtime,
            usage: &usage,
        },
        Kind::Ocr | Kind::None => ResponseView::Other,
    };
    let custom_cost_per_token = input.custom_price.as_ref().map(|rates| CustomCostPerToken {
        input_cost_per_token: rates.input_cost_per_token,
        output_cost_per_token: rates.output_cost_per_token,
        cache_read_input_token_cost: None,
        cache_creation_cost: None,
    });
    let args = CompletionArgs {
        billing: CompletionBillingInput {
            model: Some(input.model.as_str()),
            prompt: "",
            messages: &[],
            completion: "",
            call_type: input.call_type,
            custom_llm_provider: input.custom_llm_provider.as_deref(),
            region_name: input.region_name.as_deref(),
            router_model_id: None,
            custom_pricing: input.custom_pricing,
            base_model: None,
            optional_params: input.optional_params.as_ref(),
            service_tier: input.service_tier.as_deref(),
            response: Some(&view),
        },
        charge: ChargeInputs {
            passthrough_image: false,
            total_time: None,
            data_residency: None,
            vertex_location: None,
            custom_cost_per_token,
            custom_cost_per_second: None,
            logging: LoggingView {
                model: input.logging_model.as_deref(),
                params: None,
                timing: None,
            },
            response: ResponseParts {
                cost_response: None,
                image: ImageParts {
                    response: image_response,
                    quality: None,
                    image_count: None,
                    size: None,
                },
                video_usage: None,
                transcription,
                rerank: None,
                protocol: protocol.as_ref(),
                replicate: ReplicateStamps::default(),
                model_response: None,
            },
            ocr_billing: ocr_billing.as_ref(),
            vector_response: None,
            search_hidden_params: None,
            server_tool_projection: None,
            tools: ToolInputs {
                response: input::tool_response(input),
                params: input::tool_params(input),
                google_maps_requests: None,
            },
        },
    };
    let found = match input.function {
        Function::CompletionCost => completion_cost(&calculator, &args, &mut clock, target).await,
        Function::ResponseCostCalculator => {
            response_cost_calculator(&calculator, &args, input.cache_hit, &mut clock, target).await
        }
    };
    found.map_err(|error| failure("CompletionCostError", error.to_string()))
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(|error| failure("RuntimeError", error.to_string()))?;
        let mut target = Breakdown {
            refuse: input.refuse_breakdown == Some(true),
        };
        let cost = if input.refuse_breakdown == Some(true) {
            runtime.block_on(charge(input, Some(&mut target)))?
        } else {
            runtime.block_on(charge(input, None))?
        };
        Ok(json!({"cost": cost.to_f64()}))
    }
}

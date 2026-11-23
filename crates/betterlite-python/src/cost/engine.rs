use std::time::SystemTime;

use async_trait::async_trait;
use betterlite_core::cost::call_price::{CallPriceEnv, call_price};
use betterlite_core::cost::completion::args::{
    CompletionArgs, CostCalculator, SpecialBorrows, SyncImage, ToolBorrows,
};
use betterlite_core::cost::completion::lookup::CatalogLookup;
use betterlite_core::cost::completion::usage::pricing::pricing_usage;
use betterlite_core::cost::completion::{completion_cost, reading};
use betterlite_core::cost::prepare::{BillingTokenCounter, ResponseView, TypedResponse};
use betterlite_core::cost::usage::{
    BillingResponse, BillingUsage, BillingUsageError, ResponseKind, get_usage_object,
};
use betterlite_logging::fields::usage::Usage;
use betterlite_models::decisions::Json;
use betterlite_models::resolution::ResolutionContext;
use betterlite_models::tool_fees::prices::ToolPrices;
use betterlite_provider::pricing::ProviderPricing;
use betterlite_provider::pricing::grounding::{GroundingPricing, ProviderFormulas};
use betterlite_provider::pricing::image::ProviderImagePricing;
use betterlite_tokenizer::count::error::CountError;
use indexmap::IndexMap;
use rust_decimal::Decimal;

use crate::cost::billing::{CallRequest, CompletionRequest, RerankUnits};
use crate::cost::breakdown::Capture;
use crate::cost::convert::UsageRequest;
use crate::cost::error::EngineError;
use crate::cost::settings::Settings;

mod tests;

// DEFAULT_REPLICATE_GPU_PRICE_PER_SECOND: every Replicate call is priced as an A100 80GB second.
const REPLICATE_PRICE_PER_SECOND: Decimal = Decimal::from_parts(14, 0, 0, false, 4);

static PRICING: ProviderPricing = ProviderPricing;

// One cost resource: the price map, the resolver and the adjustments every call reads.
pub(crate) struct Engine {
    pub(crate) settings: Settings,
}

// What one call borrows besides the engine: the image policy, the grounding formulas and the resolution.
struct Bound<'bound> {
    image: SyncImage<'bound, ProviderImagePricing<'bound>>,
    tools: GroundingPricing<'bound, ProviderFormulas>,
    resolution: ResolutionContext<'bound>,
}

impl Engine {
    // cost_per_token: the explicit counters, or the supplied usage object, priced by B068.
    pub(crate) fn cost_per_token(
        &self,
        request: &CallRequest,
    ) -> Result<(Decimal, Decimal), EngineError> {
        let usage = match request.usage.as_ref() {
            Some(mapping) => usage_from_mapping(mapping)?,
            None => None,
        };
        let pricing = usage.as_ref().map(pricing_usage).transpose()?;
        let rerank = request
            .rerank_billed_units
            .as_ref()
            .map(RerankUnits::billed);
        let input = request.input(pricing.as_ref(), rerank.as_ref());
        let mut clock = SystemTime::now;
        let moment = reading(&mut clock)?;
        let resolution = self.settings.lookup.resolution();
        let env = CallPriceEnv {
            lookup: &self.settings.lookup,
            policy: &PRICING,
            resolution: &resolution,
            database: &self.settings.database,
            moment,
        };
        let priced = call_price(&input, &env)?;
        Ok((priced.prompt_cost, priced.completion_cost))
    }

    // completion_cost over a response, or over no response at all when there is nothing to count.
    pub(crate) async fn charge<Clock>(
        &self,
        request: &CompletionRequest,
        clock: &mut Clock,
        target: Option<&mut Capture>,
    ) -> Result<Decimal, EngineError>
    where
        Clock: FnMut() -> SystemTime + Send,
    {
        let response = request.response.as_ref().map(|wire| {
            ResponseView::Typed(TypedResponse {
                kind: wire.kind(),
                model: wire.model.as_deref(),
                usage: wire.billing_usage(),
                service_tier: wire.service_tier.as_deref(),
                hidden_params: wire.hidden_params.as_ref(),
            })
        });
        let billing = request.billing(response.as_ref());
        let charge = request.charge();
        let args = CompletionArgs { billing, charge };
        let bound = self.bound(request.model.as_deref(), clock)?;
        let counter = NoCounting;
        let calculator = self.calculator(&bound, &counter);
        Ok(completion_cost(&calculator, &args, clock, target).await?)
    }

    // get_usage_object over the usage Python read off a response, typed by the kind that carried it.
    pub(crate) fn usage_object(
        &self,
        request: &UsageRequest,
    ) -> Result<Option<Usage>, EngineError> {
        let Some(mapping) = request.usage.as_ref() else {
            return Ok(None);
        };
        let transcription;
        let decisions_ir;
        let decisions_openai;
        let kind = match request.kind {
            Some(found) => found,
            None => ResponseKind::Other,
        };
        let usage = match kind {
            ResponseKind::Transcription => {
                let text = serde_json::to_string(mapping).map_err(BillingUsageError::Json)?;
                transcription = serde_json::from_str(&text).map_err(BillingUsageError::Json)?;
                BillingUsage::Transcription(&transcription)
            }
            ResponseKind::DecisionsIr => {
                let text = serde_json::to_string(mapping).map_err(BillingUsageError::Json)?;
                decisions_ir = serde_json::from_str(&text).map_err(BillingUsageError::Json)?;
                BillingUsage::DecisionsIr(&decisions_ir)
            }
            ResponseKind::DecisionsOpenAi => {
                let text = serde_json::to_string(mapping).map_err(BillingUsageError::Json)?;
                decisions_openai = serde_json::from_str(&text).map_err(BillingUsageError::Json)?;
                BillingUsage::DecisionsOpenAi(&decisions_openai)
            }
            ResponseKind::Chat
            | ResponseKind::ChatStream
            | ResponseKind::Embedding
            | ResponseKind::Speech
            | ResponseKind::Rerank
            | ResponseKind::Image
            | ResponseKind::TextCompletion
            | ResponseKind::SendMessage
            | ResponseKind::Responses
            | ResponseKind::Other => BillingUsage::Mapping(mapping),
        };
        get_usage_object(&BillingResponse {
            kind,
            usage,
            service_tier: None,
            hidden_params: None,
        })
        .map_err(EngineError::from)
    }

    fn calculator<'call>(
        &'call self,
        bound: &'call Bound<'call>,
        counter: &'call dyn BillingTokenCounter,
    ) -> CostCalculator<
        'call,
        CatalogLookup,
        SyncImage<'call, ProviderImagePricing<'call>>,
        CatalogLookup,
    > {
        CostCalculator {
            special: SpecialBorrows {
                lookup: &self.settings.lookup,
                resolver: &self.settings.lookup,
                image_policy: &bound.image,
                special_policy: &PRICING,
                size_pattern: &self.settings.pattern,
                database: &self.settings.database,
                replicate_models: &self.settings.models,
                replicate_price: REPLICATE_PRICE_PER_SECOND,
                discount: &self.settings.discount,
                margin: &self.settings.margin,
            },
            catalog: &self.settings.lookup,
            inference: &self.settings.lookup,
            counter,
            price_policy: &PRICING,
            resolution: &bound.resolution,
            tools: ToolBorrows {
                source: &self.settings.lookup,
                policy: &bound.tools,
                prices: &ToolPrices::PINNED,
            },
        }
    }

    fn bound<'call, Clock>(
        &'call self,
        model_key: Option<&'call str>,
        clock: &mut Clock,
    ) -> Result<Bound<'call>, EngineError>
    where
        Clock: FnMut() -> SystemTime,
    {
        Ok(Bound {
            image: SyncImage {
                build: |lookup, database, moment| ProviderImagePricing {
                    lookup,
                    database,
                    moment,
                },
                lookup: &self.settings.lookup,
                database: &self.settings.database,
                moment: reading(clock)?,
            },
            tools: GroundingPricing {
                formulas: ProviderFormulas,
                model_key,
            },
            resolution: self.settings.lookup.resolution(),
        })
    }
}

fn usage_from_mapping(
    mapping: &IndexMap<String, Json>,
) -> Result<Option<Usage>, BillingUsageError> {
    get_usage_object(&BillingResponse {
        kind: ResponseKind::Other,
        usage: BillingUsage::Mapping(mapping),
        service_tier: None,
        hidden_params: None,
    })
}

// LiteLLM counts tokens only when a response has no usage; with no tokenizer, counting fails.
struct NoCounting;

#[async_trait]
impl BillingTokenCounter for NoCounting {
    async fn count_messages(&self, _model: &str, _messages: &[Json]) -> Result<usize, CountError> {
        Err(CountError::MissingInput)
    }

    async fn count_text(&self, _model: &str, text: &str) -> Result<usize, CountError> {
        if text.is_empty() {
            return Ok(0);
        }
        Err(CountError::MissingInput)
    }
}

use std::error::Error;

use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_models::price_lookup::PriceLookup;
use indexmap::IndexMap;
use rust_decimal::Decimal;

use crate::cost::context::ProviderInference;
use crate::cost::prepare::{ResponseView, TypedResponse};

mod diagnostics;
mod flow;
mod identity;
mod tests;
mod view;

// The arguments litellm.response_cost_calculator reads, without the logging object it closes over.
pub struct CostRequest<'request> {
    pub model: &'request str,
    pub cache_hit: Option<bool>,
    pub custom_llm_provider: Option<&'request str>,
    pub base_model: Option<&'request str>,
    pub call_type: &'request str,
    pub optional_params: &'request IndexMap<String, Json>,
    pub custom_pricing: bool,
    pub prompt: &'request str,
    pub standard_built_in_tools_params: &'request IndexMap<String, Json>,
    pub router_model_id: Option<&'request str>,
    pub data_residency: Option<&'request str>,
    pub vertex_location: Option<&'request str>,
    pub region_name: Option<&'request str>,
    pub response: &'request ResponseView<'request>,
}

// An adapter lives in another crate, so the port carries its failure without naming its type.
pub type PortFailure = Box<dyn Error + Send + Sync>;

// B068's calculator arrives through the constructor, so this layer never builds its own prices.
#[async_trait::async_trait]
pub trait CostCalculation: Sync {
    async fn calculate(&self, request: &CostRequest<'_>) -> Result<Decimal, PortFailure>;
}

// B1326's transform lives in betterlite-vertex, which betterlite-core cannot depend on.
pub trait GenerateContentConversion {
    fn convert(&self, body: &Json, model: &str) -> Result<Json, PortFailure>;
}

// B1304's select_region needs the catalog; an adapter outside this crate holds it.
pub trait VertexRegionNormalizer {
    fn region(&self, model: &str, configured: Option<&str>) -> String;
}

// B2207's resolve_region lives in betterlite-bedrock, injected for the same reason.
pub trait MantleRegionResolver {
    fn region(&self, litellm_params: &IndexMap<String, Json>) -> Result<String, PortFailure>;
}

// The shape a caller logs: a typed response, a terminal stream event, a mapping body or a native Gemini body.
pub enum RecordResponse<'request> {
    Typed {
        response: TypedResponse<'request>,
        background: bool,
    },
    Event(&'request Json),
    Json(&'request Json),
    GenerateContent(&'request Json),
    Absent,
}

pub struct RecordInput<'request> {
    pub response: RecordResponse<'request>,
    pub cache_hit: Option<bool>,
    pub litellm_model_name: Option<&'request str>,
    pub router_model_id: Option<&'request str>,
}

pub struct CostRecorder<'services, Inference> {
    pub calculator: &'services dyn CostCalculation,
    pub lookup: &'services (dyn PriceLookup + Sync),
    pub inference: &'services Inference,
    pub conversion: &'services dyn GenerateContentConversion,
    pub regions: &'services dyn VertexRegionNormalizer,
    pub mantle: &'services dyn MantleRegionResolver,
    pub vertex: &'services VertexSettings,
    pub ptu_cost_attribution: bool,
}

impl<Inference> CostRecorder<'_, Inference>
where
    Inference: ProviderInference,
{
    // Ports _response_cost_calculator for both the sync header and the async success path.
    pub async fn record(&self, log: &mut LoggingState, input: RecordInput<'_>) -> Option<Decimal> {
        let mut request = flow::ViewRequest {
            cache_hit: input.cache_hit,
            litellm_model_name: input.litellm_model_name,
            router_model_id: input.router_model_id,
            background: false,
        };
        match input.response {
            RecordResponse::Typed {
                response,
                background,
            } => {
                request.background = background;
                flow::CostCall {
                    recorder: self,
                    log,
                    view: &ResponseView::Typed(response),
                    request: &request,
                }
                .run()
                .await
            }
            RecordResponse::Event(event) => {
                flow::record_prepared(self, log, view::event_view(event), &request).await
            }
            RecordResponse::Json(body) => {
                request.background = view::background(body);
                flow::record_prepared(self, log, view::body_view(body), &request).await
            }
            RecordResponse::GenerateContent(body) => {
                request.background = view::background(body);
                flow::record_generate_content(flow::GenerateContent {
                    recorder: self,
                    log,
                    body,
                    request: &request,
                })
                .await
            }
            RecordResponse::Absent => {
                flow::CostCall {
                    recorder: self,
                    log,
                    view: &ResponseView::Other,
                    request: &request,
                }
                .run()
                .await
            }
        }
    }
}

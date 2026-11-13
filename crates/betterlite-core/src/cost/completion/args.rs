use betterlite_models::billing_text::ResponseText;
use betterlite_models::cost_usage::server_tool_projection::ServerToolProjection;
use betterlite_models::ocr_pricing::OcrBilling;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord};
use betterlite_models::protocol_cost::ProtocolCall;
use betterlite_models::resolution::ResolutionContext;
use betterlite_models::tool_fees::billing::{BuiltInToolBilling, BuiltInToolParams};
use betterlite_models::tool_signals::response::ToolBillingResponse;
use betterlite_openai::api::vector_store_records::records::vector::VectorStoreSearchResponse;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::cost::call_price::CustomCostPerToken;
use crate::cost::call_price::policy::ProviderPricePolicy;
use crate::cost::context::CostResponse;
use crate::cost::context::response_time::ResponseTiming;
use crate::cost::prepare::{
    BillingTokenCounter, CompletionBillingInput, PreparedBilling, RerankUnits,
};
use crate::cost::special::replicate::ReplicateStamps;
use crate::cost::special::request::{
    ImageParts, LoggingView, RealtimeSession, SpecialRequest, SpecialResponse, SpecialServices,
    TranscriptionDuration, VideoUsage,
};

// The one shared calculator: the catalog, provider policies, adjustments and tool prices one charge reads. The clock and the breakdown target are per call.
pub struct CostCalculator<'services, Resolver, ImagePolicy, Inference> {
    pub special: SpecialServices<'services, Resolver, ImagePolicy>,
    pub catalog: &'services (dyn PriceLookup + Sync),
    pub inference: &'services Inference,
    pub counter: &'services dyn BillingTokenCounter,
    pub price_policy: &'services dyn ProviderPricePolicy,
    pub resolution: &'services ResolutionContext<'services>,
    pub tools: BuiltInToolBilling<'services>,
}

// The response attributes the early-return stage reads; a branch whose response kind did not arrive keeps the empty default.
pub struct ResponseParts<'response> {
    pub cost_response: Option<&'response CostResponse<'response>>,
    pub image: ImageParts<'response>,
    pub video_usage: Option<VideoUsage<'response>>,
    pub transcription: TranscriptionDuration,
    pub rerank: Option<RerankUnits>,
    pub protocol: Option<&'response ProtocolCall>,
    pub replicate: ReplicateStamps,
    pub model_response: Option<&'response ResponseText<'response>>,
}

// What get_cost_for_built_in_tools receives besides the model: the response kind, the request's tool options and the Maps count.
pub struct ToolInputs {
    pub response: ToolBillingResponse,
    pub params: BuiltInToolParams,
    pub google_maps_requests: Option<i64>,
}

pub struct ChargeInputs<'input> {
    pub passthrough_image: bool,
    pub total_time: Option<Decimal>,
    pub data_residency: Option<&'input str>,
    pub vertex_location: Option<&'input str>,
    pub custom_cost_per_token: Option<CustomCostPerToken>,
    pub custom_cost_per_second: Option<Decimal>,
    pub logging: LoggingView<'input>,
    pub response: ResponseParts<'input>,
    pub ocr_billing: Option<&'input OcrBilling>,
    pub vector_response: Option<&'input VectorStoreSearchResponse>,
    pub search_hidden_params: Option<&'input PriceRecord>,
    pub server_tool_projection: Option<&'input ServerToolProjection>,
    pub tools: ToolInputs,
}

pub struct CompletionArgs<'args> {
    pub billing: CompletionBillingInput<'args>,
    pub charge: ChargeInputs<'args>,
}

impl<'input> ChargeInputs<'input> {
    // The early-return stage owns its request, so each prepared set of attempts gets a fresh one built from the shared parts.
    pub(super) fn special_request<'request>(
        &'request self,
        billing: &'request CompletionBillingInput<'request>,
        prepared: &'request PreparedBilling,
        moment: DateTime<Utc>,
        realtime: Option<RealtimeSession<'request>>,
    ) -> SpecialRequest<'request> {
        let timing = self.logging.timing.as_ref().map(|clock| ResponseTiming {
            start_time: clock.start_time,
            end_time: clock.end_time,
            now: clock.now,
        });
        SpecialRequest {
            call_type: prepared.call_type,
            passthrough_image: self.passthrough_image,
            prompt: billing.prompt,
            messages: billing.messages,
            optional_params: billing.optional_params,
            custom_pricing: billing.custom_pricing,
            router_model_id: billing.router_model_id,
            base_model: billing.base_model,
            selected_model: prepared.selected_model.as_deref(),
            service_tier: prepared.service_tier.as_deref(),
            data_residency: self.data_residency,
            total_time: self.total_time,
            moment,
            logging: LoggingView {
                model: self.logging.model,
                params: self.logging.params,
                timing,
            },
            response: self.special_response(realtime),
        }
    }

    fn special_response<'request>(
        &'request self,
        realtime: Option<RealtimeSession<'request>>,
    ) -> SpecialResponse<'request> {
        let parts = &self.response;
        SpecialResponse {
            cost_response: parts.cost_response,
            image: parts.image,
            video_usage: parts.video_usage.clone(),
            transcription: parts.transcription,
            rerank: parts.rerank.as_ref().map(|units| RerankUnits {
                search_units: units.search_units,
                total_tokens: units.total_tokens,
            }),
            realtime,
            protocol: parts.protocol,
            replicate: parts.replicate,
            model_response: parts.model_response,
        }
    }
}

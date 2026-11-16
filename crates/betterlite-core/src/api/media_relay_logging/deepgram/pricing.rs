use std::time::SystemTime;

use betterlite_logging::logging_state::LoggingState;
use betterlite_models::tool_fees::billing::BuiltInToolParams;
use betterlite_models::tool_signals::response::{OtherView, ToolBillingResponse};
use rust_decimal::Decimal;

use crate::api::deepgram::listen_metadata::pricing::{
    PROVIDER, addon_pricing_models, pricing_model,
};
use crate::api::deepgram::listen_metadata::target::ListenTarget;
use crate::cost::completion::args::{
    ChargeInputs, CompletionArgs, CostCalculator, ResponseParts, ToolInputs,
};
use crate::cost::completion::completion_cost;
use crate::cost::context::ProviderInference;
use crate::cost::image_dispatch::policy::{ImagePricePolicy, ImageResponseView};
use crate::cost::prepare::CompletionBillingInput;
use crate::cost::special::policy::ModelResolver;
use crate::cost::special::replicate::ReplicateStamps;
use crate::cost::special::request::{
    HiddenDuration, ImageParts, LoggingView, TranscriptionDuration,
};
use crate::routes::CallType;

// The collaborators one session's registry prices read: the shared cost calculator and the clock its charges take.
pub(super) struct ListenPricing<'borrow, 'services, Resolver, ImagePolicy, Inference, Clock> {
    pub(super) services: &'borrow CostCalculator<'services, Resolver, ImagePolicy, Inference>,
    pub(super) clock: &'borrow mut Clock,
}

impl<Resolver, ImagePolicy, Inference, Clock>
    ListenPricing<'_, '_, Resolver, ImagePolicy, Inference, Clock>
where
    Resolver: ModelResolver + Sync,
    ImagePolicy: ImagePricePolicy + Sync,
    Inference: ProviderInference + Sync,
    Clock: FnMut() -> SystemTime + Send,
{
    // The base must price for any cost to exist; an addon that fails to price contributes nothing.
    pub(super) async fn audio_cost(
        &mut self,
        billed: Decimal,
        target: &ListenTarget,
    ) -> Option<Decimal> {
        let mut total = self.registry_cost(billed, &pricing_model(target)).await?;
        for addon in addon_pricing_models(target) {
            if let Some(cost) = self.registry_cost(billed, &addon).await {
                total = total.checked_add(cost)?;
            }
        }
        Some(total)
    }

    // A failed charge logs debug and contributes nothing, the way the source swallows the exception.
    async fn registry_cost(&mut self, billed: Decimal, model: &str) -> Option<Decimal> {
        let args = transcription_args(model, billed);
        let charge = completion_cost(
            self.services,
            &args,
            &mut *self.clock,
            None::<&mut LoggingState>,
        )
        .await;
        match charge {
            Ok(cost) => Some(cost),
            Err(error) => {
                tracing::debug!(model = %model, %error, "deepgram listen passthrough has no registry price");
                None
            }
        }
    }
}

// completion_cost(call_type="transcription") for one registry entry; the billed duration rides on the response's hidden params.
fn transcription_args(model: &str, billed: Decimal) -> CompletionArgs<'_> {
    CompletionArgs {
        billing: CompletionBillingInput {
            model: Some(model),
            prompt: "",
            messages: &[],
            completion: "",
            call_type: Some(CallType::Transcription),
            custom_llm_provider: Some(PROVIDER),
            region_name: None,
            router_model_id: None,
            custom_pricing: None,
            base_model: None,
            optional_params: None,
            service_tier: None,
            response: None,
        },
        charge: ChargeInputs {
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
                transcription: TranscriptionDuration {
                    hidden: HiddenDuration::Seconds(billed),
                    response_duration: None,
                },
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
        },
    }
}

use std::time::SystemTime;

use async_trait::async_trait;
use betterlite_logging::logging_state::LoggingState;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use rust_decimal::Decimal;

use crate::api::response_synthetic_events::stamping::{CostTarget, ResponsesCostCalculator};
use crate::cost::completion::args::{CompletionArgs, CostCalculator};
use crate::cost::completion::completion_cost;
use crate::cost::context::ProviderInference;
use crate::cost::image_dispatch::policy::ImagePricePolicy;
use crate::cost::special::policy::ModelResolver;

#[async_trait]
pub trait FailurePrice: Sync {
    async fn cost(&self, response: &ResponsesApiResponse) -> Option<Decimal>;
}

#[async_trait]
impl<Calculator: ResponsesCostCalculator + Sync> FailurePrice for Calculator {
    async fn cost(&self, response: &ResponsesApiResponse) -> Option<Decimal> {
        self.response_cost(&CostTarget::Model(response))
            .ok()
            .flatten()
    }
}

// The caller prepares B068's args from this terminal response and its pinned request context.
pub struct NativePrice<'call, Resolver, ImagePolicy, Inference> {
    pub services: &'call CostCalculator<'call, Resolver, ImagePolicy, Inference>,
    pub args: &'call CompletionArgs<'call>,
    pub moment: SystemTime,
}

#[async_trait]
impl<Resolver, ImagePolicy, Inference> FailurePrice
    for NativePrice<'_, Resolver, ImagePolicy, Inference>
where
    Resolver: ModelResolver + Sync,
    ImagePolicy: ImagePricePolicy + Sync,
    Inference: ProviderInference + Sync,
{
    async fn cost(&self, _response: &ResponsesApiResponse) -> Option<Decimal> {
        completion_cost(
            self.services,
            self.args,
            &mut || self.moment,
            None::<&mut LoggingState>,
        )
        .await
        .ok()
    }
}

pub mod judge_call;
pub mod judge_prompt;
pub mod response_format;
pub mod router_call;
pub mod unmask;

use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_core::shadow_evidence::cost::PriceMap;
use betterlite_core::shadow_evidence::failure::{Location, detail};
use betterlite_core::shadow_evidence::records::CallFailure;
use betterlite_logging::logging_privacy::CallbackMetadataError;
use betterlite_models::decisions::Json;
use betterlite_models::judge_primitives::provider_qualified::ProviderCache;
use betterlite_models::resolution::ResolutionContext;
use indexmap::IndexMap;
use rand::RngExt as _;
use rand::rngs::StdRng;
use rust_decimal::Decimal;

use crate::judge_targets::router::JudgeRouter;
use crate::judge_targets::sdk::JudgeSdk;

mod tests;

pub const NO_ROUTER_ERROR: &str = "no router configured on this pod";

// The router call's fixed arguments, then the sampled request's own sampling params and tools
pub struct ShadowCall<'call> {
    pub model: &'call str,
    pub messages: Vec<Json>,
    pub metadata: &'call mut IndexMap<String, Json>,
    pub num_retries: u32,
    pub fallbacks: Vec<Json>,
    pub params: IndexMap<String, Json>,
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct ShadowRouterError {
    pub class: String,
    pub location: Option<Location<'static>>,
    pub message: String,
}

#[async_trait]
pub trait ShadowRouter: Send + Sync {
    async fn acompletion(&self, call: ShadowCall<'_>) -> Result<Json, ShadowRouterError>;
}

// The router is created lazily, so the provider answers at call time; the judge reaches the same router through B4080's trait
pub trait ShadowRouterProvider: Send + Sync {
    fn shadow_router(&self) -> Option<Arc<dyn ShadowRouter>>;
    fn judge_router(&self) -> Option<Arc<dyn JudgeRouter>>;
}

pub trait PositionCoin: Send + Sync {
    fn real_is_a(&self) -> bool;
}

pub struct UniformCoin {
    random: Mutex<StdRng>,
}

impl UniformCoin {
    #[must_use]
    pub const fn new(random: StdRng) -> Self {
        Self {
            random: Mutex::new(random),
        }
    }
}

impl PositionCoin for UniformCoin {
    fn real_is_a(&self) -> bool {
        let mut random = self.random.lock().unwrap_or_else(PoisonError::into_inner);
        random.random_bool(0.5)
    }
}

pub struct ShadowRequest<'request> {
    pub target_model: &'request str,
    pub messages: &'request [Json],
    pub shadow_params: &'request IndexMap<String, Json>,
    pub parent_metadata: &'request IndexMap<String, Json>,
}

pub struct ShadowDeps<'deps> {
    pub provider: &'deps dyn ShadowRouterProvider,
    pub prices: &'deps (dyn PriceMap + Sync),
}

pub struct JudgeRequest<'request> {
    pub judge_model: &'request str,
    pub messages: &'request [Json],
    pub real_text: &'request str,
    pub shadow_text: &'request str,
    pub tools: &'request Json,
    pub parent_metadata: &'request IndexMap<String, Json>,
}

pub struct JudgeDeps<'deps> {
    pub provider: &'deps dyn ShadowRouterProvider,
    pub coin: &'deps dyn PositionCoin,
    pub sdk: &'deps dyn JudgeSdk,
    pub cache: &'deps ProviderCache,
    pub context: &'deps ResolutionContext<'deps>,
    pub prices: &'deps (dyn PriceMap + Sync),
}

pub(crate) fn metadata_failure(source: CallbackMetadataError) -> CallFailure {
    CallFailure {
        error: detail("CallbackMetadataError", None, source),
        cost: Decimal::ZERO,
        classifier_cost: Decimal::ZERO,
    }
}

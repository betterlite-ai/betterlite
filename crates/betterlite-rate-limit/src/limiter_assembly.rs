use std::sync::{Arc, Mutex, PoisonError};

use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_config::sections::rate_limiter_settings;
use betterlite_core::api::request_destination::RequestData;
use betterlite_core::routes::CallType;
use betterlite_error::rate_limit::ProxyRateLimitError;

use crate::atomic_counter::response::ResponseDecodeError;
use crate::atomic_counter::value::CounterValueError as AtomicCounterValueError;
use crate::concurrency_gauges::{ConcurrencyGauges, GaugeError};
use crate::fail_closed::{FailClosedPolicy, RateLimitUnverifiableError};
use crate::limiter_records::{LimiterClock, ModelGroupResolver};
use crate::rate_limit_window_store::{CounterScript, WindowStore, WindowStoreError};
use crate::request_stash::RequestStashSlot;
use crate::settlement_and_rollback::{AtomicSettlement, SettlementError};
use crate::token_adjustments::store::{TokenAdjustmentError, TokenAdjustments};
use crate::windowed_admission::{WindowedAdmission, WindowedAdmissionError};

pub mod router_model_groups;
pub mod selection;
mod tests;

// the typed request body a proxy request carries, which every rate-limit hook reads
pub type RateLimitCallData = RequestData;

#[async_trait::async_trait]
pub trait CallTypeRateLimiter: Send + Sync {
    async fn async_pre_call_hook(
        &self,
        user_api_key_dict: &UserAPIKeyAuth,
        data: &RateLimitCallData,
        call_type: CallType,
        stash: &RequestStashSlot,
    ) -> Result<(), ProxyRateLimitError>;
}

pub struct RateLimiterAssembly {
    pub settings: rate_limiter_settings::Settings,
    pub clock: Arc<dyn LimiterClock>,
    pub fail_closed: FailClosedPolicy,
    pub windows: Arc<WindowStore>,
    pub check_and_increment_script: Option<Arc<dyn CounterScript>>,
    pub gauges: Arc<ConcurrencyGauges>,
    pub admission: Arc<WindowedAdmission>,
    pub adjustments: Arc<TokenAdjustments>,
    pub model_groups: Box<dyn ModelGroupResolver>,
    // LiteLLM lazy-builds its _batch_rate_limiter on the first acreate_batch; boot wires it here instead
    pub batch_limiter: Mutex<Option<Arc<dyn CallTypeRateLimiter>>>,
}

impl RateLimiterAssembly {
    #[must_use]
    pub fn batch_limiter(&self) -> Option<Arc<dyn CallTypeRateLimiter>> {
        self.batch_limiter
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub fn set_batch_limiter(&self, limiter: Arc<dyn CallTypeRateLimiter>) {
        let mut slot = self
            .batch_limiter
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        *slot = Some(limiter);
    }

    // LiteLLM builds AtomicSettlement per check from the scripts, clock and policy the handler holds
    #[must_use]
    pub fn settlement(&self) -> AtomicSettlement<'_> {
        AtomicSettlement {
            windows: self.windows.as_ref(),
            clock: self.clock.as_ref(),
            script: self.check_and_increment_script.clone(),
            refund_script: self.admission.refund_script.clone(),
            fail_closed: self.fail_closed,
            default_window_size: self.settings.window_size,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LimiterAssemblyError {
    #[error(transparent)]
    Unverifiable(#[from] RateLimitUnverifiableError),
    #[error(transparent)]
    WindowStore(#[from] WindowStoreError),
    #[error(transparent)]
    AtomicCounter(#[from] AtomicCounterValueError),
    #[error(transparent)]
    AtomicResponse(#[from] ResponseDecodeError),
    #[error(transparent)]
    Settlement(#[from] SettlementError),
    #[error(transparent)]
    Gauge(#[from] GaugeError),
    #[error(transparent)]
    Admission(#[from] WindowedAdmissionError),
    #[error(transparent)]
    TokenAdjustment(#[from] TokenAdjustmentError),
}

mod construction;
mod contract;
pub mod error;
mod logging;
mod tests;

use std::sync::Arc;
use std::time::SystemTime;

use betterlite_config::sections::xecguard_runtime::Settings as RuntimeSettings;
use reqwest::Client;

use self::error::XecGuardRuntimeError;
use crate::guardrail_base::GuardrailBase;
use crate::presidio_actions::request_data::RequestData;
use crate::presidio_response::wire::ResponseBody;
use crate::third_party_init_b::xecguard::XecGuardArgs;
use crate::xecguard_primitives::logging_hook::logging_hook;
use crate::xecguard_primitives::settings::XecGuardSettings;

type Clock = Arc<dyn Fn() -> SystemTime + Send + Sync>;

pub struct XecGuardGuard {
    base: GuardrailBase,
    settings: XecGuardSettings,
    client: Client,
    clock: Clock,
}

impl XecGuardGuard {
    pub fn new(
        args: &XecGuardArgs,
        settings: &RuntimeSettings,
        base: GuardrailBase,
        client: Client,
    ) -> Result<Self, XecGuardRuntimeError> {
        Ok(Self {
            settings: construction::resolve(args, settings, &base)?,
            base,
            client,
            clock: Arc::new(SystemTime::now),
        })
    }

    pub async fn async_logging_hook(&self, kwargs: &mut RequestData, result: &ResponseBody) {
        if let Err(error) = logging::scan_for_logging(self, kwargs, result).await {
            tracing::debug!("XecGuard logging_only swallowed exception: {error}");
        }
    }

    #[must_use]
    pub fn logging_hook(
        &self,
        kwargs: RequestData,
        result: ResponseBody,
    ) -> (RequestData, ResponseBody) {
        logging_hook(
            kwargs,
            result,
            async |hooked_kwargs: &mut RequestData, hooked_result: &mut ResponseBody| {
                self.async_logging_hook(hooked_kwargs, hooked_result).await;
                Ok::<(), XecGuardRuntimeError>(())
            },
        )
    }
}

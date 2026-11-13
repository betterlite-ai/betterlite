use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_config::sections::akto_mirroring::{Configured, UnreachableFallback};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_util::task::TaskTracker;

pub mod error;
pub mod payload;
mod tests;
pub mod transport;

use error::AktoError;
use payload::AktoInputs;

pub const HTTP_PROXY_PATH: &str = "/api/http-proxy";
pub const AKTO_CONNECTOR_NAME: &str = "litellm";
pub const SUPPORTED_EVENT_HOOKS: [&str; 2] = ["pre_call", "post_call"];

#[derive(Clone)]
pub struct AktoMirroring {
    pub base_url: String,
    pub api_key: SecretString,
    pub account_id: String,
    pub vxlan_id: String,
    pub unreachable_fallback: UnreachableFallback,
    pub timeout: Duration,
    pub client: reqwest::Client,
    pub tasks: TaskTracker,
    pub now_millis: fn() -> i64,
}

impl AktoMirroring {
    pub fn new(settings: Configured, client: reqwest::Client) -> Result<Self, AktoError> {
        if settings.api_base.is_empty() {
            return Err(AktoError::MissingBaseUrl);
        }
        if settings.api_key.expose_secret().is_empty() {
            return Err(AktoError::MissingApiKey);
        }
        Ok(Self {
            base_url: settings.api_base,
            api_key: settings.api_key,
            account_id: settings.account_id,
            vxlan_id: settings.vxlan_id,
            unreachable_fallback: settings.unreachable_fallback,
            timeout: settings.timeout,
            client,
            tasks: TaskTracker::new(),
            now_millis: current_millis,
        })
    }

    pub fn handle_unreachable(
        &self,
        inputs: AktoInputs,
        error: &AktoError,
    ) -> Result<AktoInputs, AktoError> {
        match self.unreachable_fallback {
            UnreachableFallback::FailOpen => {
                tracing::error!(error = %error, "Akto unreachable (fail-open)");
                Ok(inputs)
            }
            UnreachableFallback::FailClosed => {
                tracing::error!(error = %error, "Akto unreachable (fail-closed)");
                Err(AktoError::Unreachable)
            }
        }
    }
}

fn current_millis() -> i64 {
    let Ok(elapsed) = SystemTime::now().duration_since(UNIX_EPOCH) else {
        return 0;
    };
    i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
}

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use betterlite_auth::auth_errors::conversion::AuthFailure;
use betterlite_auth::auth_errors::handler::{AuthErrorSettings, LogSwitches};
use betterlite_auth::auth_errors::ports::{FailureHookCall, HookError};
use betterlite_auth::auth_route_patterns::normalize::RouteNormalizer;
use betterlite_config::types::Config;
use betterlite_db::database_availability::policy::allows_requests_during_outage;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};

use crate::state::shared::Shared;

// proxy_logging_obj.post_call_failure_hook, which may hand back the exception the client gets instead
#[async_trait]
pub trait AuthFailureHook: Send + Sync {
    async fn transform(&self, call: FailureHookCall<'_>) -> Result<Option<AuthFailure>, HookError>;
}

pub struct Slot {
    pub settings: AuthErrorSettings,
    // both compile fixed patterns, so None only means the build itself is broken and rejections keep their own response
    pub routes: Option<RouteNormalizer>,
    pub redactor: Option<Redactor>,
    // B012 installs the failure callbacks here; until then every rejection is answered as raised
    pub hook: OnceLock<Arc<dyn AuthFailureHook>>,
}

impl Slot {
    #[must_use]
    pub fn new(config: &Config, _shared: &Shared) -> Self {
        let switches = &config.sections.auth_errors;
        let outage = &config.sections.database_availability.allow_requests;
        Self {
            settings: AuthErrorSettings {
                log: LogSwitches {
                    client_error_tracebacks: switches.log_client_error_tracebacks,
                    key_identity: switches.log_auth_failure_key_identity,
                },
                allow_requests_on_db_unavailable: matches!(
                    allows_requests_during_outage(outage),
                    Ok(true)
                ),
                use_x_forwarded_for: config.sections.auth_request_policy.use_x_forwarded_for,
            },
            routes: RouteNormalizer::new().ok(),
            redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).ok(),
            hook: OnceLock::new(),
        }
    }
}

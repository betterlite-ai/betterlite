use std::sync::Mutex;

use async_trait::async_trait;
use betterlite_auth::auth_errors::conversion::AuthFailure;
use betterlite_auth::auth_errors::ports::{
    AuthErrorPorts, AuthLog, FailureHookCall, HookError, Level, Logger, NoNativeProvider,
    ServiceFailure,
};
use betterlite_auth::rejection_provider::{DeploymentSnapshot, ProviderResolution};
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_models::decisions::Json;
use serde_json::{Value, json};

use super::field;
use super::types::Reply;

// every port call becomes one event, in the order the handler makes them
pub(super) struct Recorder {
    pub(super) events: Mutex<Vec<Value>>,
    pub(super) reply: Reply,
    pub(super) replacement: Option<AuthFailure>,
}

impl Recorder {
    fn push(&self, event: Value) {
        if let Ok(mut events) = self.events.lock() {
            events.push(event);
        }
    }
}

#[async_trait]
impl AuthErrorPorts for Recorder {
    fn record_service_failure(&self, failure: &ServiceFailure<'_>) {
        self.push(json!({"event": "service", "call_type": failure.call_type,
            "duration": failure.duration.as_secs_f64()}));
    }

    fn log(&self, entry: AuthLog) {
        let logger = match entry.logger {
            Logger::Proxy => "LiteLLM Proxy",
            Logger::ProxyStdout => "LiteLLM Proxy.stdout",
        };
        let level = match entry.level {
            Level::Warning => "WARNING",
            Level::Error => "ERROR",
        };
        self.push(json!({"event": "log", "logger": logger, "level": level,
            "traceback": entry.traceback, "message": entry.message}));
    }

    fn seed_identity(&self, identity: &UserAPIKeyAuth, model: Option<&Json>) {
        let route = field(identity, "request_route");
        self.push(json!({"event": "seed", "request_route": route, "model": model}));
    }

    fn native_resolution(&self) -> &dyn ProviderResolution {
        &NoNativeProvider
    }

    fn router(&self) -> Option<&dyn DeploymentSnapshot> {
        None
    }

    async fn post_call_failure_hook(
        &self,
        call: FailureHookCall<'_>,
    ) -> Result<Option<AuthFailure>, HookError> {
        self.push(
            json!({"event": "hook", "error_type": call.error_type.to_string(),
            "route": call.route, "metadata": call.request_data.get("metadata"),
            "user_id": field(call.identity, "user_id")}),
        );
        match self.reply {
            Reply::Original => Ok(None),
            Reply::Replace => Ok(self.replacement.clone()),
            Reply::Raise => Err(HookError::Raised),
        }
    }
}

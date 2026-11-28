use std::collections::BTreeMap;

use async_trait::async_trait;
use betterlite_config::sections::alert_settings::types::AlertType;
use betterlite_config::sections::alert_settings::webhooks::WebhookUrl;
use betterlite_config::sections::settings::JsonValue;
use betterlite_logging::registry::phases::CallbackSelection;
use chrono::{DateTime, FixedOffset};

use crate::proxy_config_state::error::ConfigStateError;
use crate::proxy_config_state::snapshot::ConfigWithBaseline;

// LiteLLM's datetime.now(): deleted_at renders in the proxy's local wall clock without its offset
pub trait CallbackClock: Send + Sync {
    fn now(&self) -> DateTime<FixedOffset>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemCallbackClock;

impl CallbackClock for SystemCallbackClock {
    fn now(&self) -> DateTime<FixedOffset> {
        chrono::Local::now().fixed_offset()
    }
}

// B8380's save_config over the running ProxyConfigState, which the composition root owns and locks
#[async_trait]
pub trait ConfigSaver: Send + Sync {
    async fn save_config(&self, config: ConfigWithBaseline) -> Result<(), ConfigStateError>;
}

// What _get_callback_string, _callback_module_name, _is_litellm_internal_callback and _is_instance_of_configured_callback read
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeCallback {
    pub name: String,
    pub module: String,
    pub type_name: String,
    pub class_str: Option<String>,
    pub guardrail: bool,
    pub registered_integration: bool,
}

// The logging callback manager: the typed name lists, the objects behind them, and the registry lookup (names match `RuntimeCallback::name`)
pub trait RuntimeCallbacks: Send + Sync {
    fn selection(&self) -> CallbackSelection;

    fn objects(&self) -> Vec<RuntimeCallback>;

    fn class_str_for_name(&self, name: &str) -> Option<String>;
}

// SlackAlerting: the alert types that are switched on and the webhook map
pub trait AlertingState: Send + Sync {
    fn active_alert_types(&self) -> Vec<AlertType>;

    fn alert_to_webhook_url(&self) -> BTreeMap<AlertType, WebhookUrl>;
}

// llm_router.get_settings(), absent when the proxy has no router
pub trait RouterSettingsReader: Send + Sync {
    fn get_settings(&self) -> BTreeMap<String, JsonValue>;
}

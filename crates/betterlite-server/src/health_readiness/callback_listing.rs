use betterlite_config::sections::health_readiness::Settings as HealthSettings;
use betterlite_logging::registry::identity::CallbackIdentity;
use serde::Serialize;

mod tests;

// The seven module-level lists LiteLLM reads; B189's registry has no async split, so the source repeats them
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CallbackNames {
    pub callbacks: Vec<String>,
    pub input: Vec<String>,
    pub failure: Vec<String>,
    pub success: Vec<String>,
    pub async_success: Vec<String>,
    pub async_failure: Vec<String>,
    pub async_input: Vec<String>,
}

pub trait CallbackNameSource: Send + Sync {
    fn callback_names(&self) -> CallbackNames;
}

// LiteLLM branches on a plain string and on a CustomLogger object; betterlite's registered callbacks are their identities
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallbackEntry {
    Plain(String),
    Registered(CallbackIdentity),
}

#[must_use]
pub fn callback_name(callback: &CallbackEntry) -> String {
    match callback {
        CallbackEntry::Plain(name) => name.clone(),
        CallbackEntry::Registered(identity) => identity.class_name.clone(),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CallbackListingBody {
    alerting: String,
    #[serde(rename = "litellm.callbacks")]
    callbacks: Vec<String>,
    #[serde(rename = "litellm.input_callback")]
    input_callback: Vec<String>,
    #[serde(rename = "litellm.failure_callback")]
    failure_callback: Vec<String>,
    #[serde(rename = "litellm.success_callback")]
    success_callback: Vec<String>,
    #[serde(rename = "litellm._async_success_callback")]
    async_success_callback: Vec<String>,
    #[serde(rename = "litellm._async_failure_callback")]
    async_failure_callback: Vec<String>,
    #[serde(rename = "litellm._async_input_callback")]
    async_input_callback: Vec<String>,
    all_litellm_callbacks: Vec<String>,
    num_callbacks: usize,
    num_alerting: usize,
    #[serde(rename = "litellm.request_timeout")]
    request_timeout: f64,
}

#[must_use]
pub fn callback_listing(
    source: &dyn CallbackNameSource,
    settings: &HealthSettings,
) -> CallbackListingBody {
    let names = source.callback_names();
    let mut all_litellm_callbacks = names.callbacks.clone();
    all_litellm_callbacks.extend_from_slice(&names.input);
    all_litellm_callbacks.extend_from_slice(&names.failure);
    all_litellm_callbacks.extend_from_slice(&names.success);
    all_litellm_callbacks.extend_from_slice(&names.async_success);
    all_litellm_callbacks.extend_from_slice(&names.async_failure);
    all_litellm_callbacks.extend_from_slice(&names.async_input);
    let num_callbacks = all_litellm_callbacks.len();
    let num_alerting = settings.alerting.as_ref().map_or(0, Vec::len);
    CallbackListingBody {
        alerting: alerting_text(settings.alerting.as_deref()),
        callbacks: names.callbacks,
        input_callback: names.input,
        failure_callback: names.failure,
        success_callback: names.success,
        async_success_callback: names.async_success,
        async_failure_callback: names.async_failure,
        async_input_callback: names.async_input,
        all_litellm_callbacks,
        num_callbacks,
        num_alerting,
        request_timeout: settings.request_timeout,
    }
}

// Python's str() of the list, since LiteLLM puts general_settings.get("alerting") in the body as text
fn alerting_text(alerting: Option<&[String]>) -> String {
    let Some(values) = alerting else {
        return "None".to_owned();
    };
    let quoted: Vec<String> = values.iter().map(|value| format!("'{value}'")).collect();
    format!("[{}]", quoted.join(", "))
}

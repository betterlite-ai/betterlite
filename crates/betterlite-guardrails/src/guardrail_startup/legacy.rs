use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::{
    CallbackArgs, GuardrailItem, GuardrailValue,
};
use betterlite_config::sections::legacy_guardrails::Settings;

use super::error::StartupError;

const PRESIDIO: &str = "presidio";

pub trait LegacyCallbackStarter {
    fn is_started(&self, callback: &str) -> bool;

    fn start(
        &mut self,
        callbacks: &[String],
        callback_args: &CallbackArgs,
    ) -> Result<(), StartupError>;
}

#[derive(Debug, Clone, PartialEq)]
pub struct LegacyPlan {
    pub callbacks: Vec<String>,
    pub callback_args: CallbackArgs,
}

// LiteLLM passes list(set), so only the first-appearance order here is stable
#[must_use]
pub fn plan_legacy_callbacks(
    items: &[GuardrailItem],
    started: &dyn LegacyCallbackStarter,
) -> LegacyPlan {
    let mut callbacks: Vec<String> = Vec::new();
    let mut callback_args = CallbackArgs::new();
    for item in items {
        // dict.update replaces whole top-level keys, default_on or not
        callback_args.extend(item.callback_args.clone());
        if !item.default_on {
            continue;
        }
        for callback in &item.callbacks {
            if !started.is_started(callback) && !callbacks.contains(callback) {
                callbacks.push(callback.clone());
            }
            // this rewrite sits outside the started test in LiteLLM, so it also runs for a started presidio
            if item.logging_only == Some(true) && callback == PRESIDIO {
                callback_args.insert(
                    PRESIDIO.to_owned(),
                    BTreeMap::from([("logging_only".to_owned(), GuardrailValue::Boolean(true))]),
                );
            }
        }
    }
    LegacyPlan {
        callbacks,
        callback_args,
    }
}

pub fn start_legacy_guardrails(
    settings: &Settings,
    starter: &mut dyn LegacyCallbackStarter,
) -> Result<(), StartupError> {
    let plan = plan_legacy_callbacks(&settings.items, starter);
    if plan.callbacks.is_empty() {
        return Ok(());
    }
    // LiteLLM logs and re-raises, so the proxy stops
    starter
        .start(&plan.callbacks, &plan.callback_args)
        .inspect_err(|error| tracing::error!("error initializing guardrails {error}"))
}

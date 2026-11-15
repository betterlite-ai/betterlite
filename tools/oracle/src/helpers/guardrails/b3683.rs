use std::collections::BTreeMap;

use betterlite_config::sections::Section as _;
use betterlite_config::sections::guardrail_identity::enums::Role;
use betterlite_config::sections::guardrail_identity::item::{CallbackArgs, GuardrailItem};
use betterlite_config::sections::legacy_guardrails::{Raw, RawSettings, RawSpec};
use betterlite_guardrails::guardrail_startup::error::StartupError;
use betterlite_guardrails::guardrail_startup::legacy::{
    LegacyCallbackStarter, start_legacy_guardrails,
};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::helpers::{Helper, HelperResult, OrHelperError as _};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    LegacyPlan,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    function: Function,
    guardrails: Vec<IndexMap<String, RawSpec>>,
    started: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ItemOutput {
    callbacks: Vec<String>,
    default_on: bool,
    logging_only: Option<bool>,
    enabled_roles: Option<Vec<Role>>,
    callback_args: CallbackArgs,
}

#[derive(Debug, Serialize)]
struct Output {
    callbacks: Vec<String>,
    callback_args: Option<CallbackArgs>,
    by_name: BTreeMap<String, ItemOutput>,
}

// stands in for LiteLLM's replaced initialize_callbacks_on_proxy: it records the call and starts nothing
struct Recorder {
    started: Vec<String>,
    call: Option<(Vec<String>, CallbackArgs)>,
}

impl LegacyCallbackStarter for Recorder {
    fn is_started(&self, callback: &str) -> bool {
        self.started.iter().any(|name| name == callback)
    }

    fn start(
        &mut self,
        callbacks: &[String],
        callback_args: &CallbackArgs,
    ) -> Result<(), StartupError> {
        self.call = Some((callbacks.to_vec(), callback_args.clone()));
        Ok(())
    }
}

fn item_output(item: &GuardrailItem) -> ItemOutput {
    ItemOutput {
        callbacks: item.callbacks.clone(),
        default_on: item.default_on,
        logging_only: item.logging_only,
        enabled_roles: item.enabled_roles.clone(),
        callback_args: item.callback_args.clone(),
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input.function {
            Function::LegacyPlan => {
                let raw = Raw {
                    litellm_settings: RawSettings {
                        guardrails: input.guardrails.clone(),
                    },
                };
                let settings = raw
                    .convert(&BTreeMap::<String, String>::new())
                    .or_config_error()?;
                let mut recorder = Recorder {
                    started: input.started.clone(),
                    call: None,
                };
                start_legacy_guardrails(&settings, &mut recorder).or_value_error()?;
                let (mut callbacks, callback_args) = recorder
                    .call
                    .map_or((Vec::new(), None), |(started, args)| (started, Some(args)));
                // LiteLLM builds the callbacks from a set, so only the sorted list is comparable
                callbacks.sort();
                let output = Output {
                    callbacks,
                    callback_args,
                    by_name: settings
                        .by_name
                        .iter()
                        .map(|(name, item)| (name.clone(), item_output(item)))
                        .collect(),
                };
                Ok(serde_json::to_value(&output)?)
            }
        }
    }
}

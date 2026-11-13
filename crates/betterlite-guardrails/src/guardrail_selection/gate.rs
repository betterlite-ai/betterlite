use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::{LitellmParamsMode, ModeTarget};
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use super::error::GuardrailSelectionError;
use super::metadata::{
    global_guardrails_disabled, opted_out_global_guardrails, requested_guardrails,
};
use crate::guardrail_base::GuardrailBase;

type Data = BTreeMap<String, GuardrailValue>;

const fn hook_text(event_type: GuardrailEventHooks) -> &'static str {
    match event_type {
        GuardrailEventHooks::PreCall => "pre_call",
        GuardrailEventHooks::PostCall => "post_call",
        GuardrailEventHooks::DuringCall => "during_call",
        GuardrailEventHooks::LoggingOnly => "logging_only",
        GuardrailEventHooks::PreMcpCall => "pre_mcp_call",
        GuardrailEventHooks::DuringMcpCall => "during_mcp_call",
        GuardrailEventHooks::PostMcpCall => "post_mcp_call",
        GuardrailEventHooks::RealtimeInputTranscription => "realtime_input_transcription",
    }
}

fn target_holds(target: &ModeTarget, hook: &str) -> bool {
    match target {
        ModeTarget::One(name) => name == hook,
        ModeTarget::Many(names) => names.iter().any(|name| name == hook),
    }
}

pub trait GuardrailSelection {
    fn should_run_guardrail(
        &self,
        data: &Data,
        event_type: GuardrailEventHooks,
        suppressed: &BTreeSet<String>,
    ) -> Result<bool, GuardrailSelectionError>;

    fn event_hook_is_event_type(&self, event_type: GuardrailEventHooks) -> bool;

    fn is_requested_guardrail(&self, requested: &[GuardrailValue]) -> bool;

    fn dynamic_request_body_params(&self, data: &Data) -> Result<Data, GuardrailSelectionError>;
}

impl GuardrailSelection for GuardrailBase {
    // LiteLLM's should_run_guardrail. `suppressed` is what an auto router turned off for this request
    fn should_run_guardrail(
        &self,
        data: &Data,
        event_type: GuardrailEventHooks,
        suppressed: &BTreeSet<String>,
    ) -> Result<bool, GuardrailSelectionError> {
        let settings = &self.settings;
        if let Some(name) = &settings.guardrail_name
            && suppressed.contains(name)
        {
            return Ok(false);
        }
        if settings.default_on {
            if let Some(name) = &settings.guardrail_name
                && opted_out_global_guardrails(data).contains(&name.as_str())
            {
                return Ok(false);
            }
            if global_guardrails_disabled(data) {
                return Ok(false);
            }
        }
        if !settings.default_on
            && settings.event_hook.is_some()
            && event_type != GuardrailEventHooks::LoggingOnly
            && !self.is_requested_guardrail(requested_guardrails(data)?)
        {
            return Ok(false);
        }
        if !self.event_hook_is_event_type(event_type) {
            return Ok(false);
        }
        if let Some(LitellmParamsMode::Tagged(_)) = &settings.event_hook {
            return Err(GuardrailSelectionError::TagMode {
                guardrail_name: settings.guardrail_name.clone(),
            });
        }
        Ok(true)
    }

    // LiteLLM's _event_hook_is_event_type
    fn event_hook_is_event_type(&self, event_type: GuardrailEventHooks) -> bool {
        let hook = hook_text(event_type);
        match &self.settings.event_hook {
            None => true,
            Some(LitellmParamsMode::One(name)) => name == hook,
            Some(LitellmParamsMode::Many(names)) => names.iter().any(|name| name == hook),
            Some(LitellmParamsMode::Tagged(mode)) => mode
                .tags
                .values()
                .chain(mode.default.iter())
                .any(|target| target_holds(target, hook)),
        }
    }

    // LiteLLM's _guardrail_is_in_requested_guardrails
    fn is_requested_guardrail(&self, requested: &[GuardrailValue]) -> bool {
        let Some(name) = &self.settings.guardrail_name else {
            return false;
        };
        requested.iter().any(|entry| match entry {
            GuardrailValue::Text(text) => text == name,
            GuardrailValue::Object(options) => options.contains_key(name),
            GuardrailValue::Null
            | GuardrailValue::Boolean(_)
            | GuardrailValue::SignedInteger(_)
            | GuardrailValue::UnsignedInteger(_)
            | GuardrailValue::Decimal(_)
            | GuardrailValue::Array(_) => false,
        })
    }

    // LiteLLM's get_guardrail_dynamic_request_body_params, without the premium check
    fn dynamic_request_body_params(&self, data: &Data) -> Result<Data, GuardrailSelectionError> {
        let requested = requested_guardrails(data)?;
        let Some(name) = &self.settings.guardrail_name else {
            return Ok(BTreeMap::new());
        };
        for entry in requested {
            let GuardrailValue::Object(options) = entry else {
                continue;
            };
            let Some(params) = options.get(name) else {
                continue;
            };
            let not_object = GuardrailSelectionError::DynamicParamsNotObject {
                guardrail_name: name.clone(),
            };
            return match params {
                GuardrailValue::Null => Ok(BTreeMap::new()),
                GuardrailValue::Object(fields) => match fields.get("extra_body") {
                    None | Some(GuardrailValue::Null) => Ok(BTreeMap::new()),
                    Some(GuardrailValue::Object(extra_body)) => Ok(extra_body.clone()),
                    Some(_) => Err(not_object),
                },
                GuardrailValue::Boolean(_)
                | GuardrailValue::SignedInteger(_)
                | GuardrailValue::UnsignedInteger(_)
                | GuardrailValue::Decimal(_)
                | GuardrailValue::Text(_)
                | GuardrailValue::Array(_) => Err(not_object),
            };
        }
        Ok(BTreeMap::new())
    }
}

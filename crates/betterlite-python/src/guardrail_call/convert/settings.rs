use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::{GuardrailFlag, GuardrailValue};
use betterlite_config::sections::guardrail_params::mode::{LitellmParamsMode, Mode, ModeTarget};
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_guardrails::guardrail_base::settings::{GuardrailSettings, GuardrailSwitches};
use serde::{Deserialize, Serialize};

use crate::guardrail_call::error::GuardrailCallError;

const TAGS: &str = "tags";
const DEFAULT: &str = "default";

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct SettingsJson {
    pub(crate) guardrail_name: Option<String>,
    pub(crate) supported_event_hooks: Option<Vec<GuardrailEventHooks>>,
    pub(crate) event_hook: Option<GuardrailValue>,
    pub(crate) default_on: GuardrailFlag,
    pub(crate) mask_request_content: GuardrailFlag,
    pub(crate) mask_response_content: GuardrailFlag,
    pub(crate) violation_message_template: Option<String>,
    pub(crate) end_session_after_n_fails: Option<i64>,
    pub(crate) on_violation: Option<String>,
    pub(crate) realtime_violation_message: Option<String>,
    pub(crate) on_sensitive_data: Option<String>,
    pub(crate) sensitive_data_route_to_model: Option<String>,
    pub(crate) sticky_session_routing: GuardrailFlag,
    pub(crate) run_in_parallel: GuardrailFlag,
    pub(crate) scan_raw_request: GuardrailFlag,
    pub(crate) only_scan_new_messages: GuardrailFlag,
    pub(crate) timeout: Option<f64>,
    pub(crate) switches: SwitchesJson,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct SwitchesJson {
    pub(crate) use_native_during_call_hook: GuardrailFlag,
    pub(crate) use_native_lifecycle_hooks: GuardrailFlag,
    pub(crate) records_own_information: GuardrailFlag,
    pub(crate) uses_apply_guardrail_interface: GuardrailFlag,
    pub(crate) deny_text_on_error: GuardrailFlag,
}

impl Default for SettingsJson {
    fn default() -> Self {
        Self {
            guardrail_name: None,
            supported_event_hooks: None,
            event_hook: None,
            default_on: false,
            mask_request_content: false,
            mask_response_content: false,
            violation_message_template: None,
            end_session_after_n_fails: None,
            on_violation: None,
            realtime_violation_message: None,
            on_sensitive_data: None,
            sensitive_data_route_to_model: None,
            sticky_session_routing: true,
            run_in_parallel: false,
            scan_raw_request: false,
            only_scan_new_messages: false,
            timeout: None,
            switches: SwitchesJson::default(),
        }
    }
}

impl From<&GuardrailSettings> for SettingsJson {
    fn from(settings: &GuardrailSettings) -> Self {
        Self {
            guardrail_name: settings.guardrail_name.clone(),
            supported_event_hooks: settings.supported_event_hooks.clone(),
            event_hook: settings.event_hook.as_ref().map(mode_to_value),
            default_on: settings.default_on,
            mask_request_content: settings.mask_request_content,
            mask_response_content: settings.mask_response_content,
            violation_message_template: settings.violation_message_template.clone(),
            end_session_after_n_fails: settings.end_session_after_n_fails,
            on_violation: settings.on_violation.clone(),
            realtime_violation_message: settings.realtime_violation_message.clone(),
            on_sensitive_data: settings.on_sensitive_data.clone(),
            sensitive_data_route_to_model: settings.sensitive_data_route_to_model.clone(),
            sticky_session_routing: settings.sticky_session_routing,
            run_in_parallel: settings.run_in_parallel,
            scan_raw_request: settings.scan_raw_request,
            only_scan_new_messages: settings.only_scan_new_messages,
            timeout: settings.timeout,
            switches: SwitchesJson::from(&settings.switches),
        }
    }
}

impl From<&GuardrailSwitches> for SwitchesJson {
    fn from(switches: &GuardrailSwitches) -> Self {
        Self {
            use_native_during_call_hook: switches.use_native_during_call_hook,
            use_native_lifecycle_hooks: switches.use_native_lifecycle_hooks,
            records_own_information: switches.records_own_information,
            uses_apply_guardrail_interface: switches.uses_apply_guardrail_interface,
            deny_text_on_error: switches.deny_text_on_error,
        }
    }
}

impl SettingsJson {
    pub(crate) fn into_settings(self) -> Result<GuardrailSettings, GuardrailCallError> {
        Ok(GuardrailSettings {
            guardrail_name: self.guardrail_name,
            supported_event_hooks: self.supported_event_hooks,
            event_hook: mode_from_value(self.event_hook)?,
            default_on: self.default_on,
            mask_request_content: self.mask_request_content,
            mask_response_content: self.mask_response_content,
            violation_message_template: self.violation_message_template,
            end_session_after_n_fails: self.end_session_after_n_fails,
            on_violation: self.on_violation,
            realtime_violation_message: self.realtime_violation_message,
            on_sensitive_data: self.on_sensitive_data,
            sensitive_data_route_to_model: self.sensitive_data_route_to_model,
            sticky_session_routing: self.sticky_session_routing,
            run_in_parallel: self.run_in_parallel,
            scan_raw_request: self.scan_raw_request,
            only_scan_new_messages: self.only_scan_new_messages,
            timeout: self.timeout,
            switches: GuardrailSwitches {
                use_native_during_call_hook: self.switches.use_native_during_call_hook,
                use_native_lifecycle_hooks: self.switches.use_native_lifecycle_hooks,
                records_own_information: self.switches.records_own_information,
                uses_apply_guardrail_interface: self.switches.uses_apply_guardrail_interface,
                deny_text_on_error: self.switches.deny_text_on_error,
            },
        })
    }
}

pub(crate) fn mode_to_value(mode: &LitellmParamsMode) -> GuardrailValue {
    match mode {
        LitellmParamsMode::One(name) => GuardrailValue::Text(name.clone()),
        LitellmParamsMode::Many(names) => target_to_value(ModeTarget::Many(names.clone())),
        LitellmParamsMode::Tagged(mode) => {
            let tags = mode
                .tags
                .iter()
                .map(|(tag, target)| (tag.clone(), target_to_value(target.clone())))
                .collect();
            let mut fields = BTreeMap::from([(TAGS.to_owned(), GuardrailValue::Object(tags))]);
            if let Some(default) = &mode.default {
                fields.insert(DEFAULT.to_owned(), target_to_value(default.clone()));
            }
            GuardrailValue::Object(fields)
        }
    }
}

fn target_to_value(target: ModeTarget) -> GuardrailValue {
    match target {
        ModeTarget::One(name) => GuardrailValue::Text(name),
        ModeTarget::Many(names) => {
            GuardrailValue::Array(names.into_iter().map(GuardrailValue::Text).collect())
        }
    }
}

fn mode_from_value(
    value: Option<GuardrailValue>,
) -> Result<Option<LitellmParamsMode>, GuardrailCallError> {
    match value {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(GuardrailValue::Text(name)) => Ok(Some(LitellmParamsMode::One(name))),
        Some(GuardrailValue::Array(items)) => {
            Ok(Some(LitellmParamsMode::Many(names_from_values(items)?)))
        }
        Some(GuardrailValue::Object(fields)) => {
            Ok(Some(LitellmParamsMode::Tagged(mode_from_fields(fields)?)))
        }
        Some(
            GuardrailValue::Boolean(_)
            | GuardrailValue::SignedInteger(_)
            | GuardrailValue::UnsignedInteger(_)
            | GuardrailValue::Decimal(_),
        ) => Err(GuardrailCallError::EventHookShape),
    }
}

fn mode_from_fields(fields: BTreeMap<String, GuardrailValue>) -> Result<Mode, GuardrailCallError> {
    let mut tags = None;
    let mut default = None;
    for (key, value) in fields {
        match key.as_str() {
            TAGS => tags = Some(tags_from_value(value)?),
            DEFAULT => default = default_from_value(value)?,
            _unknown => return Err(GuardrailCallError::ModeKey { name: key }),
        }
    }
    let Some(tags) = tags else {
        return Err(GuardrailCallError::EventHookShape);
    };
    Ok(Mode { tags, default })
}

fn tags_from_value(
    value: GuardrailValue,
) -> Result<BTreeMap<String, ModeTarget>, GuardrailCallError> {
    let GuardrailValue::Object(entries) = value else {
        return Err(GuardrailCallError::EventHookShape);
    };
    let mut tags = BTreeMap::new();
    for (tag, target) in entries {
        tags.insert(tag, target_from_value(target)?);
    }
    Ok(tags)
}

fn default_from_value(value: GuardrailValue) -> Result<Option<ModeTarget>, GuardrailCallError> {
    match value {
        GuardrailValue::Null => Ok(None),
        other => target_from_value(other).map(Some),
    }
}

fn target_from_value(value: GuardrailValue) -> Result<ModeTarget, GuardrailCallError> {
    match value {
        GuardrailValue::Text(name) => Ok(ModeTarget::One(name)),
        GuardrailValue::Array(items) => Ok(ModeTarget::Many(names_from_values(items)?)),
        GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Object(_) => Err(GuardrailCallError::EventHookShape),
    }
}

fn names_from_values(items: Vec<GuardrailValue>) -> Result<Vec<String>, GuardrailCallError> {
    let mut names = Vec::with_capacity(items.len());
    for item in items {
        let GuardrailValue::Text(name) = item else {
            return Err(GuardrailCallError::EventHookShape);
        };
        names.push(name);
    }
    Ok(names)
}

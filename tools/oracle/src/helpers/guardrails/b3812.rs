use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_initializers::tool_permission::ToolPermissionArgs;
use betterlite_guardrails::tool_permission::ToolPermissionGuardrail;
use betterlite_guardrails::tool_permission::config::ToolPermissionConfigModel;
use betterlite_guardrails::tool_permission_requests::pre_call_hook;
use serde::{Deserialize, Serialize};

use crate::helpers::{Helper, HelperError, HelperResult, OrHelperError as _};

const GUARDRAIL_NAME: &str = "tool-permissions";
const FORWARDED_KEYS: [&str; 4] = ["tools", "functions", "tool_choice", "function_call"];

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    rules: Vec<BTreeMap<String, GuardrailValue>>,
    default_action: String,
    on_disallowed_action: String,
    event_hook: String,
    default_on: bool,
    #[serde(default)]
    violation_message_template: Option<String>,
    data: BTreeMap<String, GuardrailValue>,
}

#[derive(Debug, Serialize)]
struct Blocked {
    status: u16,
    detail: BTreeMap<String, GuardrailValue>,
}

#[derive(Debug, Serialize)]
struct Output {
    blocked: Option<Blocked>,
    request: BTreeMap<String, GuardrailValue>,
    applied_guardrails: Option<Vec<String>>,
}

// The names the hook recorded in the bucket LiteLLM writes proxy state to
fn applied_guardrails(data: &BTreeMap<String, GuardrailValue>) -> Option<Vec<String>> {
    let key = if data.contains_key("litellm_metadata") {
        "litellm_metadata"
    } else {
        "metadata"
    };
    let Some(GuardrailValue::Object(bucket)) = data.get(key) else {
        return None;
    };
    let Some(GuardrailValue::Array(names)) = bucket.get("applied_guardrails") else {
        return None;
    };
    Some(
        names
            .iter()
            .filter_map(|name| match name {
                GuardrailValue::Text(text) => Some(text.clone()),
                GuardrailValue::Null
                | GuardrailValue::Boolean(_)
                | GuardrailValue::SignedInteger(_)
                | GuardrailValue::UnsignedInteger(_)
                | GuardrailValue::Decimal(_)
                | GuardrailValue::Array(_)
                | GuardrailValue::Object(_) => None,
            })
            .collect(),
    )
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let settings = ToolPermissionConfigModel {
            optional_params: None,
            rules: None,
            default_action: input.default_action.clone(),
            on_disallowed_action: input.on_disallowed_action.clone(),
        }
        .resolve()
        .or_value_error()?;
        let args = ToolPermissionArgs {
            name: GUARDRAIL_NAME.to_owned(),
            event_hook: LitellmParamsMode::One(input.event_hook.clone()),
            rules: Some(input.rules.clone()),
            default_action: settings.default_action,
            on_disallowed_action: settings.on_disallowed_action,
            default_on: input.default_on,
            violation_message_template: input.violation_message_template.clone(),
        };
        let guardrail = ToolPermissionGuardrail::new(args).or_value_error()?;
        let mut data = input.data.clone();
        let blocked = match pre_call_hook(&guardrail, &mut data, &BTreeSet::new()) {
            Ok(()) => None,
            Err(GuardrailFailure::HttpObject { status, detail }) => Some(Blocked {
                status,
                detail: detail.into_iter().collect(),
            }),
            Err(other) => {
                return Err(Box::new(HelperError {
                    class: "GuardrailFailure".to_owned(),
                    status: None,
                    error_type: None,
                    param: None,
                    code: None,
                    message: other.to_string(),
                }));
            }
        };
        let request = data
            .iter()
            .filter(|(key, _value)| FORWARDED_KEYS.contains(&key.as_str()))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        let output = Output {
            blocked,
            request,
            applied_guardrails: applied_guardrails(&data),
        };
        Ok(serde_json::to_value(output)?)
    }
}

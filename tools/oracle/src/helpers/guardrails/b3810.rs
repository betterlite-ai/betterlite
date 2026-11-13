use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity as _;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_guardrails::guardrail_initializers::tool_permission::ToolPermissionArgs;
use betterlite_guardrails::tool_permission::ToolPermissionGuardrail;
use betterlite_guardrails::tool_permission::config::{
    ToolPermissionConfigModel, default_action_text, disallowed_action_text, parse_default_action,
    parse_disallowed_action,
};
use betterlite_guardrails::tool_permission::rule::ToolPermissionRule;
use serde::{Deserialize, Serialize};

use crate::helpers::{Helper, HelperResult, OrHelperError as _};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    rules: Vec<BTreeMap<String, GuardrailValue>>,
    default_action: String,
    on_disallowed_action: String,
}

#[derive(Debug, Serialize)]
struct Output {
    rules: Vec<ToolPermissionRule>,
    default_action: &'static str,
    on_disallowed_action: &'static str,
    ui_friendly_name: &'static str,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let args = ToolPermissionArgs {
            name: String::new(),
            event_hook: LitellmParamsMode::One("pre_call".to_owned()),
            rules: Some(input.rules.clone()),
            default_action: parse_default_action(&input.default_action).or_value_error()?,
            on_disallowed_action: parse_disallowed_action(&input.on_disallowed_action)
                .or_value_error()?,
            default_on: false,
            violation_message_template: None,
        };
        let guardrail = ToolPermissionGuardrail::new(args).or_value_error()?;
        let output = Output {
            rules: guardrail.rules,
            default_action: default_action_text(guardrail.args.default_action),
            on_disallowed_action: disallowed_action_text(guardrail.args.on_disallowed_action),
            ui_friendly_name: ToolPermissionConfigModel::ui_friendly_name(),
        };
        Ok(serde_json::to_value(output)?)
    }
}

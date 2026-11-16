use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_guardrails::guardrail_initializers::tool_permission::ToolPermissionArgs;
use betterlite_guardrails::tool_permission::ToolPermissionGuardrail;
use betterlite_guardrails::tool_permission::config::{
    ToolPermissionConfigModel, default_action_text, disallowed_action_text,
};
use betterlite_guardrails::tool_permission::rule::ToolPermissionRule;
use serde::{Deserialize, Serialize};

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

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
            name: String::new(),
            event_hook: LitellmParamsMode::One("pre_call".to_owned()),
            rules: Some(input.rules.clone()),
            default_action: settings.default_action,
            on_disallowed_action: settings.on_disallowed_action,
            default_on: false,
            violation_message_template: None,
        };
        let guardrail = ToolPermissionGuardrail::new(args).or_value_error()?;
        let output = Output {
            rules: guardrail.rules,
            default_action: default_action_text(guardrail.args.default_action),
            on_disallowed_action: disallowed_action_text(guardrail.args.on_disallowed_action),
        };
        Ok(serde_json::to_value(output)?)
    }
}

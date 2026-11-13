use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_guardrails::guardrail_base::GuardrailBase;
use betterlite_guardrails::guardrail_base::settings::{GuardrailSettings, GuardrailSwitches};
use betterlite_guardrails::guardrail_selection::error::GuardrailSelectionError;
use betterlite_guardrails::guardrail_selection::gate::GuardrailSelection as _;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Function {
    ShouldRunGuardrail,
    EventHookIsEventType,
    DynamicRequestBodyParams,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    function: Function,
    guardrail_name: Option<String>,
    event_hook: Option<LitellmParamsMode>,
    default_on: bool,
    data: BTreeMap<String, GuardrailValue>,
    event_type: Option<GuardrailEventHooks>,
    suppressed: Option<Vec<String>>,
}

fn refusal(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn guardrail_of(input: &Input) -> GuardrailBase {
    GuardrailBase {
        settings: GuardrailSettings {
            guardrail_name: input.guardrail_name.clone(),
            event_hook: input.event_hook.clone(),
            default_on: input.default_on,
            supported_event_hooks: None,
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
            switches: GuardrailSwitches::default(),
        },
        mode: None,
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let base = guardrail_of(input);
        let event_type = || {
            input
                .event_type
                .ok_or_else(|| refusal("InputError", "the case needs an event_type".to_owned()))
        };
        let result = match input.function {
            Function::EventHookIsEventType => {
                Value::Bool(base.event_hook_is_event_type(event_type()?))
            }
            Function::ShouldRunGuardrail => {
                let suppressed: BTreeSet<String> =
                    input.suppressed.iter().flatten().cloned().collect();
                match base.should_run_guardrail(&input.data, event_type()?, &suppressed) {
                    Ok(runs) => Value::Bool(runs),
                    Err(GuardrailSelectionError::TagMode { .. }) => json!("rejected"),
                    Err(error) => {
                        return Err(refusal("GuardrailSelectionError", error.to_string()));
                    }
                }
            }
            Function::DynamicRequestBodyParams => {
                let params = base
                    .dynamic_request_body_params(&input.data)
                    .map_err(|error| refusal("GuardrailSelectionError", error.to_string()))?;
                json!(params)
            }
        };
        Ok(json!({ "result": result }))
    }
}

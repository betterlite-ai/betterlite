use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::{LitellmParamsMode, Mode, ModeTarget};
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_guardrails::guardrail_base::GuardrailBase;
use betterlite_guardrails::guardrail_base::advisory::inject_advisory_message;
use betterlite_guardrails::guardrail_base::failure::{
    GuardrailFailure, ModifyResponseFailure, SensitiveDataRouteFailure,
};
use betterlite_guardrails::guardrail_base::mask::mask_content_in_string;
use betterlite_guardrails::guardrail_base::session::session_id_from_request;
use betterlite_guardrails::guardrail_base::settings::{GuardrailSettings, GuardrailSwitches};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FailureKind {
    Http,
    ModifyResponse,
    Raised,
    BlockedPii,
    SensitiveRoute,
    Other,
}

#[derive(Debug, Deserialize)]
pub(crate) struct HookArgs {
    supported: Vec<GuardrailEventHooks>,
    event_hook: Option<String>,
    event_hooks: Option<Vec<String>>,
    mode_tags: Option<BTreeMap<String, ModeTarget>>,
    mode_default: Option<ModeTarget>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct InterventionArgs {
    kind: FailureKind,
    status: u16,
}

#[derive(Debug, Deserialize)]
pub(crate) struct FailureTextArgs {
    kind: FailureKind,
    guardrail_name: Option<String>,
    message: String,
    wrap_default_message: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ViolationArgs {
    template: Option<String>,
    default: String,
    context: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PassthroughArgs {
    guardrail_name: Option<String>,
    message: String,
    request_data: BTreeMap<String, GuardrailValue>,
    detection_info: Option<BTreeMap<String, GuardrailValue>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AdvisoryArgs {
    data: BTreeMap<String, GuardrailValue>,
    message: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MaskArgs {
    content: String,
    mask: String,
    start: i64,
    end: i64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SessionArgs {
    request_data: BTreeMap<String, GuardrailValue>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    ValidateHook(HookArgs),
    Intervention(InterventionArgs),
    FailureText(FailureTextArgs),
    Violation(ViolationArgs),
    Passthrough(PassthroughArgs),
    Advisory(AdvisoryArgs),
    Mask(MaskArgs),
    Session(SessionArgs),
}

fn failure(
    kind: &FailureKind,
    status: u16,
    guardrail_name: Option<String>,
    message: &str,
    wrap: bool,
) -> GuardrailFailure {
    match kind {
        FailureKind::Http => GuardrailFailure::Http {
            status,
            detail: String::new(),
        },
        FailureKind::ModifyResponse => {
            GuardrailFailure::ModifyResponse(Box::new(ModifyResponseFailure {
                message: message.to_owned(),
                model: "gpt-4".to_owned(),
                request_data: BTreeMap::new(),
                guardrail_name,
                detection_info: BTreeMap::new(),
                original_response: None,
            }))
        }
        FailureKind::Raised => GuardrailFailure::GuardrailRaised {
            guardrail_name,
            message: message.to_owned(),
            wrap_default_message: wrap,
            status_code: 400,
            blocked_content: false,
        },
        FailureKind::BlockedPii => GuardrailFailure::BlockedPii {
            entity_type: "EMAIL".to_owned(),
            guardrail_name,
            status_code: 400,
        },
        FailureKind::SensitiveRoute => {
            GuardrailFailure::SensitiveDataRoute(Box::new(SensitiveDataRouteFailure {
                route_to_model: "local".to_owned(),
                session_id: "session".to_owned(),
                guardrail_name,
                detection_info: BTreeMap::new(),
                message: Some(message.to_owned()).filter(|text| !text.is_empty()),
                sticky_session_routing: true,
            }))
        }
        FailureKind::Other => GuardrailFailure::Other("plain failure".to_owned()),
    }
}

fn settings_with(
    guardrail_name: Option<String>,
    template: Option<String>,
    hooks: (Option<Vec<GuardrailEventHooks>>, Option<LitellmParamsMode>),
) -> GuardrailSettings {
    GuardrailSettings {
        guardrail_name,
        supported_event_hooks: hooks.0,
        event_hook: hooks.1,
        default_on: false,
        mask_request_content: false,
        mask_response_content: false,
        violation_message_template: template,
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
        switches: GuardrailSwitches {
            use_native_during_call_hook: false,
            use_native_lifecycle_hooks: false,
            records_own_information: false,
            uses_apply_guardrail_interface: false,
            deny_text_on_error: false,
        },
    }
}

fn base_with(guardrail_name: Option<String>, template: Option<String>) -> GuardrailBase {
    GuardrailBase {
        settings: settings_with(guardrail_name, template, (None, None)),
        mode: None,
    }
}

fn hook_accepted(args: &HookArgs) -> bool {
    let requested = match (&args.mode_tags, &args.event_hooks, &args.event_hook) {
        (Some(tags), _, _) => Some(LitellmParamsMode::Tagged(Mode {
            tags: tags.clone(),
            default: args.mode_default.clone(),
        })),
        (None, Some(names), _) => Some(LitellmParamsMode::Many(names.clone())),
        (None, None, Some(name)) => Some(LitellmParamsMode::One(name.clone())),
        (None, None, None) => None,
    };
    let hooks = (Some(args.supported.clone()), requested);
    GuardrailBase::new(settings_with(None, None, hooks), true).is_ok()
}

fn passthrough(args: &PassthroughArgs) -> Value {
    let base = base_with(args.guardrail_name.clone(), None);
    let raised = base.passthrough_failure(
        &args.message,
        &args.request_data,
        args.detection_info.clone(),
        None,
    );
    if let GuardrailFailure::ModifyResponse(details) = raised {
        return json!({
            "message": details.message,
            "model": details.model,
            "guardrail_name": details.guardrail_name,
            "detection_info": details.detection_info,
        });
    }
    Value::Null
}

fn violation(args: &ViolationArgs) -> String {
    let pairs: Vec<(&str, &str)> = args
        .context
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    base_with(None, args.template.clone()).render_violation_message(&args.default, &pairs)
}

fn advisory(args: &AdvisoryArgs) -> Value {
    let mut changed = args.data.clone();
    let applied = inject_advisory_message(&mut changed, &args.message);
    json!({ "applied": applied, "data": changed })
}

fn failure_text(args: &FailureTextArgs) -> String {
    failure(
        &args.kind,
        400,
        args.guardrail_name.clone(),
        &args.message,
        args.wrap_default_message,
    )
    .to_string()
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let answer = match input {
            Input::ValidateHook(args) => Value::Bool(hook_accepted(args)),
            Input::Intervention(args) => Value::Bool(
                failure(&args.kind, args.status, None, "blocked", true).is_intervention(),
            ),
            Input::FailureText(args) => Value::String(failure_text(args)),
            Input::Violation(args) => Value::String(violation(args)),
            Input::Passthrough(args) => passthrough(args),
            Input::Advisory(args) => advisory(args),
            Input::Mask(args) => Value::String(mask_content_in_string(
                &args.content,
                &args.mask,
                args.start,
                args.end,
            )),
            Input::Session(args) => {
                session_id_from_request(&args.request_data).map_or(Value::Null, Value::String)
            }
        };
        Ok(answer)
    }
}

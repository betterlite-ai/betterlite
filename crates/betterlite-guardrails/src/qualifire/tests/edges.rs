use std::collections::BTreeMap;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use reqwest::StatusCode;
use secrecy::{ExposeSecret as _, SecretString};
use tracing::subscriber::with_default;

use super::{
    FixedSecrets, NoSecrets, args, build, converted_rows, environment, inputs, json, message,
    params, payload_value, resolve, row, row_from_json, text,
};
use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::qualifire::convert;
use crate::qualifire::error::QualifireError;
use crate::qualifire::request::{self, CheckRequest};
use crate::qualifire::settings::{self, Check, OnFlagged};
use crate::qualifire::wire::BlockDetail;
use crate::qualifire::{QualifireGuardrail, SUPPORTED_EVENT_HOOKS};

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

const TOOL_NOTICE: &str = "tool_selection_quality_check enabled but no available_tools provided";

fn plan_log(tool_check: bool, tools: Option<&GuardrailValue>) -> String {
    let mut args = args();
    args.tool_selection_quality_check = Some(tool_check);
    let guardrail = build(&args);
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(move || writer.clone())
        .finish();
    with_default(subscriber, || {
        let check = CheckRequest {
            rows: &[row("hi")],
            output: None,
            evaluation_id: None,
            assertions: &[],
            available_tools: tools,
        };
        guardrail.plan(&check).unwrap();
    });
    String::from_utf8(captured.0.lock().unwrap().clone()).unwrap()
}

#[test]
fn the_tool_check_without_tools_logs_that_it_is_skipped() {
    assert!(plan_log(true, None).contains(TOOL_NOTICE));
}

#[test]
fn no_tool_check_logs_nothing_about_tools() {
    assert!(!plan_log(false, None).contains(TOOL_NOTICE));
}

#[test]
fn the_tool_check_with_tools_logs_nothing_about_tools() {
    let tools = json(r#"[{"name": "lookup"}]"#);
    assert!(!plan_log(true, Some(&tools)).contains(TOOL_NOTICE));
}

#[test]
fn tools_are_not_sent_when_the_tool_check_is_off() {
    let guardrail = build(&args());
    let tools = json(r#"[{"name": "lookup"}]"#);
    let check = CheckRequest {
        rows: &[row("hi")],
        output: None,
        evaluation_id: None,
        assertions: &[],
        available_tools: Some(&tools),
    };
    let plan = guardrail.plan(&check).unwrap();
    assert_eq!(
        payload_value(&plan.payload()),
        json(r#"{"messages": [{"role": "user", "content": "hi"}], "prompt_injections": true}"#)
    );
}

#[test]
fn update_with_an_empty_on_flagged_goes_back_to_blocking() {
    let mut args = args();
    args.on_flagged = Some("monitor".to_owned());
    let mut guardrail = build(&args);
    guardrail
        .update_params(&params(r#", "on_flagged": """#))
        .unwrap();
    assert_eq!(guardrail.settings.on_flagged, OnFlagged::Block);
}

#[test]
fn update_with_a_null_on_flagged_goes_back_to_blocking() {
    let mut args = args();
    args.on_flagged = Some("monitor".to_owned());
    let mut guardrail = build(&args);
    guardrail
        .update_params(&params(r#", "on_flagged": null"#))
        .unwrap();
    assert_eq!(guardrail.settings.on_flagged, OnFlagged::Block);
}

#[test]
fn update_rejects_an_on_flagged_that_is_not_text() {
    let mut guardrail = build(&args());
    let error = guardrail
        .update_params(&params(r#", "on_flagged": 5"#))
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Qualifire guardrail does not support on_flagged='5'; only 'block' and 'monitor' are supported."
    );
}

#[test]
fn update_takes_the_new_api_base_and_key() {
    let mut guardrail = build(&args());
    guardrail
        .update_params(&params(
            r#", "api_base": "https://new.test", "api_key": "new-key""#,
        ))
        .unwrap();
    assert_eq!(guardrail.settings.api_base, "https://new.test");
    assert_eq!(
        guardrail.settings.api_key.unwrap().expose_secret(),
        "new-key"
    );
}

#[test]
fn update_takes_the_timeout_and_defaults_it_when_absent() {
    let mut guardrail = build(&args());
    guardrail
        .update_params(&params(r#", "timeout": 2.5"#))
        .unwrap();
    assert_eq!(guardrail.settings.timeout, Duration::from_millis(2500));
    guardrail.update_params(&params("")).unwrap();
    assert_eq!(guardrail.settings.timeout, settings::DEFAULT_TIMEOUT);
}

#[test]
fn update_rejects_a_negative_timeout() {
    let mut guardrail = build(&args());
    let result = guardrail.update_params(&params(r#", "timeout": -1"#));
    assert!(matches!(result, Err(QualifireError::InvalidTimeout { .. })));
}

#[test]
fn update_copies_every_check_flag() {
    let mut guardrail = build(&args());
    guardrail
        .update_params(&params(
            r#", "prompt_injections": true, "hallucinations_check": true, "grounding_check": true, "pii_check": true, "content_moderation_check": true, "tool_selection_quality_check": true"#,
        ))
        .unwrap();
    assert_eq!(
        guardrail.settings.checks,
        [
            Check::PromptInjections,
            Check::Hallucinations,
            Check::Grounding,
            Check::Pii,
            Check::ContentModeration,
            Check::ToolSelectionQuality
        ]
        .into_iter()
        .collect()
    );
}

#[test]
fn update_leaves_the_checks_empty_without_flags() {
    let mut guardrail = build(&args());
    guardrail.update_params(&params("")).unwrap();
    assert!(guardrail.settings.checks.is_empty());
}

#[test]
fn the_base_carries_the_name_hooks_and_default_flag() {
    let mut args = args();
    args.default_on = true;
    args.event_hook = LitellmParamsMode::One("post_call".to_owned());
    let guardrail = build(&args);
    let base = &guardrail.base.settings;
    assert_eq!(base.guardrail_name.as_deref(), Some("qualifire"));
    assert_eq!(
        base.supported_event_hooks.as_deref(),
        Some(SUPPORTED_EVENT_HOOKS.as_slice())
    );
    assert_eq!(
        base.event_hook,
        Some(LitellmParamsMode::One("post_call".to_owned()))
    );
    assert!(base.default_on);
    assert!(base.switches.uses_apply_guardrail_interface);
}

#[test]
fn an_empty_name_leaves_the_guardrail_unnamed() {
    let mut args = args();
    args.name = String::new();
    assert_eq!(build(&args).base.settings.guardrail_name, None);
}

#[test]
fn blocked_is_a_bad_request_and_other_failures_are_server_errors() {
    let detail = BlockDetail {
        error: "Violated guardrail policy".to_owned(),
        qualifire_response: crate::qualifire::wire::QualifireResponse {
            score: GuardrailValue::Null,
            status: GuardrailValue::Null,
        },
    };
    assert_eq!(
        QualifireError::Blocked { detail }.status_code(),
        StatusCode::BAD_REQUEST
    );
    let failure = QualifireError::HttpStatus {
        status: StatusCode::BAD_GATEWAY,
    };
    assert_eq!(failure.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[test]
fn the_on_flagged_message_escapes_quotes_backslashes_and_control_characters() {
    let error = QualifireError::UnsupportedOnFlagged("a'b\\c\nd\re\tf".to_owned());
    assert_eq!(
        error.to_string(),
        "Qualifire guardrail does not support on_flagged='a\\'b\\\\c\\nd\\re\\tf'; only 'block' and 'monitor' are supported."
    );
}

#[test]
fn a_tool_call_that_is_not_an_object_is_an_error() {
    let row = row_from_json(r#"{"role": "assistant", "tool_calls": ["bad"]}"#);
    let error = convert::api_message(&row).unwrap_err();
    assert_eq!(
        error.to_string(),
        "the Qualifire guardrail cannot read tool_calls in the request messages"
    );
}

#[test]
fn a_tool_call_keeps_object_arguments_and_defaults_the_rest() {
    let row = row_from_json(
        r#"{"role": "assistant", "tool_calls": [{"function": {"arguments": {"city": "Paris"}}}, {}]}"#,
    );
    let converted = converted_rows(&[row]);
    let calls = converted[0].tool_calls.as_ref().unwrap();
    assert_eq!(calls[0].id, "");
    assert_eq!(calls[0].name, "");
    assert_eq!(
        calls[0].arguments,
        BTreeMap::from([("city".to_owned(), text("Paris"))])
    );
    assert_eq!(calls[1].arguments, BTreeMap::new());
}

#[test]
fn a_null_or_missing_role_defaults_to_user() {
    let converted = converted_rows(&[
        row_from_json(r#"{"content": "hi"}"#),
        row_from_json(r#"{"role": null, "content": "hi"}"#),
    ]);
    assert_eq!(converted[0].role, "user");
    assert_eq!(converted[1].role, "user");
}

#[test]
fn a_wrong_typed_role_is_an_error() {
    let error = convert::api_message(&row_from_json(r#"{"role": 5}"#)).unwrap_err();
    assert_eq!(
        error.to_string(),
        "the Qualifire guardrail cannot read role in the request messages"
    );
}

#[test]
fn a_text_part_with_a_wrong_typed_text_is_an_error() {
    let content = json(r#"[{"type": "text", "text": 5}]"#);
    let rows = convert::message_rows(&[message("user", content)]);
    let error = convert::api_message(&rows[0]).unwrap_err();
    assert_eq!(
        error.to_string(),
        "the Qualifire guardrail cannot read text in the request messages"
    );
}

#[test]
fn a_text_part_without_text_adds_an_empty_line() {
    let content = json(r#"[{"type": "text"}, {"type": "text", "text": "b"}]"#);
    let rows = convert::message_rows(&[message("user", content)]);
    assert_eq!(converted_rows(&rows)[0].content, "\nb");
}

#[test]
fn tools_that_are_not_a_list_are_an_error() {
    let tools = json(r#"{"name": "lookup"}"#);
    let error = convert::api_tools(Some(&tools)).unwrap_err();
    assert_eq!(
        error.to_string(),
        "the Qualifire guardrail cannot read tools in the request messages"
    );
}

#[test]
fn a_tool_that_is_not_an_object_is_an_error() {
    let tools = json(r#"["lookup"]"#);
    assert!(convert::api_tools(Some(&tools)).is_err());
}

#[test]
fn a_function_tool_with_a_wrong_typed_function_is_an_error() {
    let tools = json(r#"[{"type": "function", "function": 5}]"#);
    let error = convert::api_tools(Some(&tools)).unwrap_err();
    assert_eq!(
        error.to_string(),
        "the Qualifire guardrail cannot read function in the request messages"
    );
}

#[test]
fn a_function_tool_without_a_function_is_skipped() {
    let tools = json(r#"[{"type": "function"}, {"type": "function", "function": null}]"#);
    assert_eq!(convert::api_tools(Some(&tools)).unwrap(), None);
}

#[test]
fn a_tool_definition_with_a_wrong_typed_name_is_an_error() {
    let tools = json(r#"[{"name": 5}]"#);
    assert!(convert::api_tools(Some(&tools)).is_err());
}

#[test]
fn dynamic_params_of_the_wrong_type_are_errors() {
    for (field, value) in [
        ("evaluation_id", "5"),
        ("assertions", r#""rule""#),
        ("assertions", "[5]"),
        ("on_flagged", "5"),
    ] {
        let dynamic = BTreeMap::from([(field.to_owned(), json(value))]);
        let error = request::overrides(&dynamic).unwrap_err();
        assert_eq!(
            error.to_string(),
            format!("the Qualifire guardrail request option {field} has an invalid value")
        );
    }
}

#[test]
fn dynamic_params_that_are_null_are_ignored() {
    let dynamic = BTreeMap::from([
        ("evaluation_id".to_owned(), GuardrailValue::Null),
        ("on_flagged".to_owned(), GuardrailValue::Null),
    ]);
    let overrides = request::overrides(&dynamic).unwrap();
    assert_eq!(overrides.evaluation_id, None);
    assert_eq!(overrides.on_flagged, None);
}

#[test]
fn the_last_user_message_wins_even_when_it_is_not_text() {
    let rows = vec![
        row("first"),
        row_from_json(r#"{"role": "user", "content": 5}"#),
    ];
    assert_eq!(request::invocation_input(&rows), "");
}

#[test]
fn texts_beside_structured_messages_do_not_replace_them() {
    let mut inputs = inputs();
    inputs.structured_messages = Some(vec![message("user", text("from inputs"))]);
    inputs.texts = Some(vec!["a text".to_owned()]);
    for input_type in [GuardrailInputType::Request, GuardrailInputType::Response] {
        let (rows, _output) = request::select_rows(&inputs, &BTreeMap::new(), input_type)
            .unwrap()
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get("content"), Some(&text("from inputs")));
        assert_eq!(rows[0].get("role"), Some(&text("user")));
    }
}

#[test]
fn a_response_keeps_the_last_text_as_output_beside_structured_messages() {
    let mut inputs = inputs();
    inputs.structured_messages = Some(vec![message("user", text("from inputs"))]);
    inputs.texts = Some(vec!["first".to_owned(), "last".to_owned()]);
    let (_rows, output) =
        request::select_rows(&inputs, &BTreeMap::new(), GuardrailInputType::Response)
            .unwrap()
            .unwrap();
    assert_eq!(output.as_deref(), Some("last"));
}

#[test]
fn request_messages_that_are_not_a_list_are_an_error() {
    let request_data = BTreeMap::from([("messages".to_owned(), text("hi"))]);
    let error =
        request::select_rows(&inputs(), &request_data, GuardrailInputType::Request).unwrap_err();
    assert_eq!(
        error.to_string(),
        "the Qualifire guardrail cannot read messages in the request messages"
    );
}

#[test]
fn a_null_request_messages_value_selects_nothing() {
    let request_data = BTreeMap::from([("messages".to_owned(), GuardrailValue::Null)]);
    let selected =
        request::select_rows(&inputs(), &request_data, GuardrailInputType::Request).unwrap();
    assert!(selected.is_none());
}

#[test]
fn a_request_message_that_is_not_an_object_is_an_error() {
    let request_data = BTreeMap::from([("messages".to_owned(), json(r#"["hi"]"#))]);
    let result = request::select_rows(&inputs(), &request_data, GuardrailInputType::Request);
    assert!(matches!(result, Err(QualifireError::InvalidField { .. })));
}

#[test]
fn settings_use_the_environment_when_the_secret_is_empty() {
    let secrets = FixedSecrets(BTreeMap::from([(
        "QUALIFIRE_API_KEY".to_owned(),
        SecretString::from(String::new()),
    )]));
    let env = environment(&[("QUALIFIRE_API_KEY", "environment-key")]);
    let resolved = resolve(&args(), &secrets, &env);
    assert_eq!(resolved.api_key.unwrap().expose_secret(), "environment-key");
}

#[test]
fn settings_treat_an_empty_evaluation_id_as_unset() {
    let mut args = args();
    args.evaluation_id = Some(String::new());
    let resolved = resolve(&args, &NoSecrets, &environment(&[]));
    assert_eq!(resolved.evaluation_id, None);
    assert_eq!(
        resolved.checks,
        [Check::PromptInjections].into_iter().collect()
    );
}

#[test]
fn settings_enable_each_check_from_its_flag() {
    let mut args = args();
    args.prompt_injections = Some(true);
    args.hallucinations_check = Some(true);
    args.grounding_check = Some(true);
    args.pii_check = Some(true);
    args.content_moderation_check = Some(true);
    args.tool_selection_quality_check = Some(true);
    let resolved = resolve(&args, &NoSecrets, &environment(&[]));
    assert_eq!(resolved.checks.len(), 6);
}

#[test]
fn the_guardrail_is_built_from_its_arguments() {
    let mut args = args();
    args.timeout = Some(3.0);
    args.on_flagged = Some("monitor".to_owned());
    let guardrail = QualifireGuardrail::new(
        &args,
        &NoSecrets,
        &BTreeMap::new(),
        reqwest::Client::default(),
    )
    .unwrap();
    assert_eq!(guardrail.settings.timeout, Duration::from_secs(3));
    assert_eq!(guardrail.settings.on_flagged, OnFlagged::Monitor);
}

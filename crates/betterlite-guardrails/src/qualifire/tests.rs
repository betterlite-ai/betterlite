#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::decode::decode_litellm_params;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_http::proxy::base_builder;
use secrecy::{ExposeSecret as _, SecretString};

use super::convert::{self, ApiMessage, ApiTool, ApiToolCall};
use super::init::QualifireArgs;
use super::request::{self, CheckRequest};
use super::settings::{self, Check, OnFlagged, QualifireSettings, SecretLookup};
use super::wire::{BlockDetail, Payload, QualifireResponse, Verdict};
use super::{QualifireGuardrail, guardrail_settings};
use crate::generic_guardrail_contracts::{
    GuardrailApiInputs, GuardrailInputType, GuardrailMessage,
};
use crate::guardrail_base::GuardrailBase;

struct NoSecrets;

impl SecretLookup for NoSecrets {
    fn secret(&self, _name: &str) -> Option<SecretString> {
        None
    }
}

struct FixedSecrets(BTreeMap<String, SecretString>);

impl SecretLookup for FixedSecrets {
    fn secret(&self, name: &str) -> Option<SecretString> {
        self.0.get(name).cloned()
    }
}

fn args() -> QualifireArgs {
    QualifireArgs {
        api_key: None,
        api_base: None,
        evaluation_id: None,
        prompt_injections: None,
        hallucinations_check: None,
        grounding_check: None,
        pii_check: None,
        content_moderation_check: None,
        tool_selection_quality_check: None,
        assertions: None,
        on_flagged: Some("block".to_owned()),
        name: "qualifire".to_owned(),
        event_hook: LitellmParamsMode::One("pre_call".to_owned()),
        default_on: false,
        timeout: None,
    }
}

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn resolve(
    args: &QualifireArgs,
    secrets: &dyn SecretLookup,
    env: &BTreeMap<String, String>,
) -> QualifireSettings {
    settings::resolve(args, secrets, env).unwrap()
}

fn build(args: &QualifireArgs) -> QualifireGuardrail {
    QualifireGuardrail {
        base: GuardrailBase::new(guardrail_settings(args), true).unwrap(),
        settings: resolve(args, &NoSecrets, &BTreeMap::new()),
        client: base_builder().build().unwrap(),
    }
}

fn params(extra: &str) -> LitellmParams {
    let body = format!(r#"{{"guardrail": "qualifire", "mode": "pre_call"{extra}}}"#);
    decode_litellm_params(body.as_bytes()).unwrap()
}

fn text(value: &str) -> GuardrailValue {
    GuardrailValue::Text(value.to_owned())
}

fn json(value: &str) -> GuardrailValue {
    serde_json::from_str(value).unwrap()
}

fn inputs() -> GuardrailApiInputs {
    GuardrailApiInputs {
        texts: None,
        images: None,
        tools: None,
        tool_calls: None,
        structured_messages: None,
        model: None,
        stream_holdback_chars: None,
    }
}

fn message(role: &str, content: GuardrailValue) -> GuardrailMessage {
    GuardrailMessage {
        role: role.to_owned(),
        extra: BTreeMap::from([("content".to_owned(), content)]),
    }
}

fn row(content: &str) -> BTreeMap<String, GuardrailValue> {
    BTreeMap::from([
        ("role".to_owned(), text("user")),
        ("content".to_owned(), text(content)),
    ])
}

fn row_from_json(value: &str) -> BTreeMap<String, GuardrailValue> {
    match json(value) {
        GuardrailValue::Object(object) => object,
        _ => panic!("the test row is not a JSON object"),
    }
}

fn payload_value(payload: &Payload<'_>) -> GuardrailValue {
    json(&serde_json::to_string(payload).unwrap())
}

#[test]
fn settings_take_the_argument_first() {
    let mut args = args();
    args.api_key = Some(SecretString::from("argument-key".to_owned()));
    args.api_base = Some("https://argument.test".to_owned());
    let secrets = FixedSecrets(BTreeMap::from([
        (
            "QUALIFIRE_API_KEY".to_owned(),
            SecretString::from("secret-key".to_owned()),
        ),
        (
            "QUALIFIRE_BASE_URL".to_owned(),
            SecretString::from("https://secret.test".to_owned()),
        ),
    ]));
    let env = environment(&[
        ("QUALIFIRE_API_KEY", "environment-key"),
        ("QUALIFIRE_BASE_URL", "https://environment.test"),
    ]);
    let resolved = resolve(&args, &secrets, &env);
    assert_eq!(resolved.api_key.unwrap().expose_secret(), "argument-key");
    assert_eq!(resolved.api_base, "https://argument.test");
}

#[test]
fn settings_fall_back_to_the_secret_before_the_environment() {
    let secrets = FixedSecrets(BTreeMap::from([
        (
            "QUALIFIRE_API_KEY".to_owned(),
            SecretString::from("secret-key".to_owned()),
        ),
        (
            "QUALIFIRE_BASE_URL".to_owned(),
            SecretString::from("https://secret.test".to_owned()),
        ),
    ]));
    let env = environment(&[
        ("QUALIFIRE_API_KEY", "environment-key"),
        ("QUALIFIRE_BASE_URL", "https://environment.test"),
    ]);
    let resolved = resolve(&args(), &secrets, &env);
    assert_eq!(resolved.api_key.unwrap().expose_secret(), "secret-key");
    assert_eq!(resolved.api_base, "https://secret.test");
}

#[test]
fn settings_fall_back_to_the_environment() {
    let env = environment(&[
        ("QUALIFIRE_API_KEY", "environment-key"),
        ("QUALIFIRE_BASE_URL", "https://environment.test"),
    ]);
    let resolved = resolve(&args(), &NoSecrets, &env);
    assert_eq!(resolved.api_key.unwrap().expose_secret(), "environment-key");
    assert_eq!(resolved.api_base, "https://environment.test");
}

#[test]
fn settings_ignore_an_empty_environment_value() {
    let env = environment(&[("QUALIFIRE_API_KEY", ""), ("QUALIFIRE_BASE_URL", "")]);
    let resolved = resolve(&args(), &NoSecrets, &env);
    assert_eq!(resolved.api_key, None);
    assert_eq!(resolved.api_base, settings::DEFAULT_API_BASE);
}

#[test]
fn settings_default_the_api_base_and_timeout() {
    let resolved = resolve(&args(), &NoSecrets, &BTreeMap::new());
    assert_eq!(resolved.api_base, "https://proxy.qualifire.ai");
    assert_eq!(resolved.timeout, settings::DEFAULT_TIMEOUT);
    assert_eq!(resolved.on_flagged, OnFlagged::Block);
}

#[test]
fn settings_reject_an_unknown_on_flagged() {
    let mut args = args();
    args.on_flagged = Some("warn".to_owned());
    let error = settings::resolve(&args, &NoSecrets, &BTreeMap::new()).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Qualifire guardrail does not support on_flagged='warn'; only 'block' and 'monitor' are supported."
    );
}

#[test]
fn settings_accept_monitor() {
    let mut args = args();
    args.on_flagged = Some("monitor".to_owned());
    let resolved = resolve(&args, &NoSecrets, &BTreeMap::new());
    assert_eq!(resolved.on_flagged, OnFlagged::Monitor);
}

#[test]
fn settings_enable_prompt_injections_when_nothing_else_is_set() {
    let resolved = resolve(&args(), &NoSecrets, &BTreeMap::new());
    assert_eq!(resolved.checks, BTreeSet::from([Check::PromptInjections]));
}

#[test]
fn settings_keep_prompt_injections_off_when_a_check_is_set() {
    let mut args = args();
    args.pii_check = Some(true);
    let resolved = resolve(&args, &NoSecrets, &BTreeMap::new());
    assert_eq!(resolved.checks, BTreeSet::from([Check::Pii]));
}

#[test]
fn settings_keep_prompt_injections_off_when_assertions_exist() {
    let mut args = args();
    args.assertions = Some(vec!["be nice".to_owned()]);
    let resolved = resolve(&args, &NoSecrets, &BTreeMap::new());
    assert!(resolved.checks.is_empty());
    assert_eq!(resolved.assertions, vec!["be nice".to_owned()]);
}

#[test]
fn settings_keep_prompt_injections_off_with_an_evaluation_id() {
    let mut args = args();
    args.evaluation_id = Some("evaluation-1".to_owned());
    let resolved = resolve(&args, &NoSecrets, &BTreeMap::new());
    assert!(resolved.checks.is_empty());
    assert_eq!(resolved.evaluation_id.as_deref(), Some("evaluation-1"));
}

#[test]
fn settings_keep_an_explicit_prompt_injection() {
    let mut args = args();
    args.prompt_injections = Some(true);
    args.pii_check = Some(false);
    let resolved = resolve(&args, &NoSecrets, &BTreeMap::new());
    assert_eq!(resolved.checks, BTreeSet::from([Check::PromptInjections]));
}

#[test]
fn update_rejects_an_unknown_on_flagged_and_changes_nothing() {
    let mut args = args();
    args.on_flagged = Some("monitor".to_owned());
    let mut guardrail = build(&args);
    let error = guardrail
        .update_params(&params(r#", "on_flagged": "banana""#))
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Qualifire guardrail does not support on_flagged='banana'; only 'block' and 'monitor' are supported."
    );
    assert_eq!(guardrail.settings.on_flagged, OnFlagged::Monitor);
}

#[test]
fn update_takes_the_new_on_flagged() {
    let mut guardrail = build(&args());
    guardrail
        .update_params(&params(r#", "on_flagged": "monitor""#))
        .unwrap();
    assert_eq!(guardrail.settings.on_flagged, OnFlagged::Monitor);
}

#[test]
fn update_without_on_flagged_goes_back_to_blocking() {
    let mut args = args();
    args.on_flagged = Some("monitor".to_owned());
    let mut guardrail = build(&args);
    guardrail.update_params(&params("")).unwrap();
    assert_eq!(guardrail.settings.on_flagged, OnFlagged::Block);
}

#[test]
fn update_copies_the_check_flags_and_assertions() {
    let mut guardrail = build(&args());
    guardrail
        .update_params(&params(
            r#", "pii_check": true, "assertions": ["be brief"], "evaluation_id": "e1""#,
        ))
        .unwrap();
    assert_eq!(guardrail.settings.checks, BTreeSet::from([Check::Pii]));
    assert_eq!(guardrail.settings.assertions, vec!["be brief".to_owned()]);
    assert_eq!(guardrail.settings.evaluation_id.as_deref(), Some("e1"));
}

#[test]
fn update_keeps_the_api_base_when_the_params_have_none() {
    let mut args = args();
    args.api_base = Some("https://argument.test".to_owned());
    let mut guardrail = build(&args);
    guardrail.update_params(&params("")).unwrap();
    assert_eq!(guardrail.settings.api_base, "https://argument.test");
}

#[test]
fn messages_join_list_content_text_parts_with_newlines() {
    let content = json(r#"[{"type": "text", "text": "first"}, "second", {"type": "image_url"}]"#);
    let rows = convert::message_rows(&[message("user", content)]);
    let converted = convert::api_messages(&rows).unwrap();
    assert_eq!(converted[0].content, "first\nsecond");
}

#[test]
fn messages_turn_other_content_into_json_text() {
    let rows = convert::message_rows(&[message("user", json("42"))]);
    let converted = convert::api_messages(&rows).unwrap();
    assert_eq!(converted[0].content, "42");
}

#[test]
fn messages_default_a_missing_content_to_an_empty_string() {
    let rows = convert::message_rows(&[GuardrailMessage {
        role: "user".to_owned(),
        extra: BTreeMap::new(),
    }]);
    let converted = convert::api_messages(&rows).unwrap();
    assert_eq!(converted[0].content, "");
    assert_eq!(converted[0].role, "user");
}

#[test]
fn messages_copy_a_set_tool_call_id() {
    let row = BTreeMap::from([
        ("role".to_owned(), text("tool")),
        ("content".to_owned(), text("done")),
        ("tool_call_id".to_owned(), text("call-1")),
    ]);
    let converted = convert::api_messages(&[row]).unwrap();
    assert_eq!(converted[0].tool_call_id.as_deref(), Some("call-1"));
}

#[test]
fn messages_parse_string_tool_call_arguments() {
    let row = row_from_json(
        r#"{"role": "assistant", "content": "", "tool_calls": [{"id": "call-1", "function": {"name": "lookup", "arguments": "{\"city\": \"Paris\"}"}}]}"#,
    );
    let converted = convert::api_messages(&[row]).unwrap();
    let calls = converted[0].tool_calls.as_ref().unwrap();
    assert_eq!(
        calls[0],
        ApiToolCall {
            id: "call-1".to_owned(),
            name: "lookup".to_owned(),
            arguments: BTreeMap::from([("city".to_owned(), text("Paris"))]),
        }
    );
}

#[test]
fn messages_keep_a_failed_tool_call_argument_parse_as_an_empty_object() {
    let row = row_from_json(
        r#"{"role": "assistant", "content": "", "tool_calls": [{"id": "call-1", "function": {"name": "lookup", "arguments": "not json"}}]}"#,
    );
    let converted = convert::api_messages(&[row]).unwrap();
    let calls = converted[0].tool_calls.as_ref().unwrap();
    assert_eq!(calls[0].arguments, BTreeMap::new());
}

#[test]
fn messages_keep_no_tool_calls_for_an_empty_list() {
    let row = BTreeMap::from([
        ("role".to_owned(), text("assistant")),
        ("content".to_owned(), text("hi")),
        ("tool_calls".to_owned(), GuardrailValue::Array(Vec::new())),
    ]);
    let converted = convert::api_messages(&[row]).unwrap();
    assert_eq!(converted[0].tool_calls, None);
}

#[test]
fn tools_convert_the_function_shape() {
    let tools = json(
        r#"[{"type": "function", "function": {"name": "lookup", "description": "find", "parameters": {"type": "object"}}}]"#,
    );
    let converted = convert::api_tools(Some(&tools)).unwrap().unwrap();
    assert_eq!(
        converted,
        vec![ApiTool {
            name: "lookup".to_owned(),
            description: "find".to_owned(),
            parameters: BTreeMap::from([("type".to_owned(), text("object"))]),
        }]
    );
}

#[test]
fn tools_convert_the_bare_shape() {
    let tools = json(r#"[{"name": "lookup"}]"#);
    let converted = convert::api_tools(Some(&tools)).unwrap().unwrap();
    assert_eq!(converted.len(), 1);
    assert_eq!(converted[0].name, "lookup");
    assert_eq!(converted[0].description, "");
    assert_eq!(converted[0].parameters, BTreeMap::new());
}

#[test]
fn tools_are_absent_without_any_convertible_entry() {
    let empty = GuardrailValue::Array(Vec::new());
    assert_eq!(convert::api_tools(None).unwrap(), None);
    assert_eq!(convert::api_tools(Some(&empty)).unwrap(), None);
    let unrelated = json(r#"[{"type": "other"}]"#);
    assert_eq!(convert::api_tools(Some(&unrelated)).unwrap(), None);
}

#[test]
fn overrides_use_present_non_empty_dynamic_params() {
    let dynamic = BTreeMap::from([
        ("evaluation_id".to_owned(), text("e1")),
        ("assertions".to_owned(), json(r#"["be nice"]"#)),
        ("on_flagged".to_owned(), text("monitor")),
    ]);
    let overrides = request::overrides(&dynamic).unwrap();
    assert_eq!(overrides.evaluation_id.as_deref(), Some("e1"));
    assert_eq!(overrides.assertions, Some(vec!["be nice".to_owned()]));
    assert_eq!(overrides.on_flagged, Some(OnFlagged::Monitor));
}

#[test]
fn overrides_ignore_empty_dynamic_params() {
    let dynamic = BTreeMap::from([
        ("evaluation_id".to_owned(), text("")),
        ("assertions".to_owned(), GuardrailValue::Array(Vec::new())),
        ("on_flagged".to_owned(), text("")),
    ]);
    let overrides = request::overrides(&dynamic).unwrap();
    assert_eq!(overrides.evaluation_id, None);
    assert_eq!(overrides.assertions, None);
    assert_eq!(overrides.on_flagged, None);
}

#[test]
fn overrides_leave_an_unknown_on_flagged_blocking() {
    let dynamic = BTreeMap::from([("on_flagged".to_owned(), text("banana"))]);
    let overrides = request::overrides(&dynamic).unwrap();
    assert_eq!(overrides.on_flagged, Some(OnFlagged::Block));
}

#[test]
fn invocation_input_takes_the_last_user_message_content() {
    let rows = vec![
        row("first"),
        BTreeMap::from([
            ("role".to_owned(), text("user")),
            ("content".to_owned(), text("second")),
        ]),
        BTreeMap::from([
            ("role".to_owned(), text("assistant")),
            ("content".to_owned(), text("answer")),
        ]),
    ];
    assert_eq!(request::invocation_input(&rows), "second");
}

#[test]
fn invocation_input_stays_empty_for_a_list_content() {
    let rows = vec![BTreeMap::from([
        ("role".to_owned(), text("user")),
        (
            "content".to_owned(),
            json(r#"[{"type": "text", "text": "hi"}]"#),
        ),
    ])];
    assert_eq!(request::invocation_input(&rows), "");
}

#[test]
fn rows_prefer_structured_messages() {
    let mut inputs = inputs();
    inputs.structured_messages = Some(vec![message("user", text("from inputs"))]);
    let request_data = BTreeMap::from([(
        "messages".to_owned(),
        json(r#"[{"role": "user", "content": "from request"}]"#),
    )]);
    let (rows, output) = request::select_rows(&inputs, &request_data, GuardrailInputType::Request)
        .unwrap()
        .unwrap();
    assert_eq!(output, None);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get("content"), Some(&text("from inputs")));
}

#[test]
fn rows_fall_back_to_request_messages() {
    let mut inputs = inputs();
    inputs.structured_messages = Some(Vec::new());
    let request_data = BTreeMap::from([(
        "messages".to_owned(),
        json(r#"[{"role": "user", "content": "from request"}]"#),
    )]);
    let (rows, _output) = request::select_rows(&inputs, &request_data, GuardrailInputType::Request)
        .unwrap()
        .unwrap();
    assert_eq!(rows[0].get("content"), Some(&text("from request")));
}

#[test]
fn response_rows_become_one_assistant_message_from_the_last_text() {
    let mut inputs = inputs();
    inputs.texts = Some(vec!["first".to_owned(), "last".to_owned()]);
    let (rows, output) =
        request::select_rows(&inputs, &BTreeMap::new(), GuardrailInputType::Response)
            .unwrap()
            .unwrap();
    assert_eq!(output.as_deref(), Some("last"));
    assert_eq!(rows[0].get("role"), Some(&text("assistant")));
    assert_eq!(rows[0].get("content"), Some(&text("last")));
}

#[test]
fn request_rows_become_one_user_message_from_the_last_text() {
    let mut inputs = inputs();
    inputs.texts = Some(vec!["first".to_owned(), "last".to_owned()]);
    let (rows, output) =
        request::select_rows(&inputs, &BTreeMap::new(), GuardrailInputType::Request)
            .unwrap()
            .unwrap();
    assert_eq!(output, None);
    assert_eq!(rows[0].get("role"), Some(&text("user")));
    assert_eq!(rows[0].get("content"), Some(&text("last")));
}

#[test]
fn no_messages_or_texts_selects_nothing() {
    let selected =
        request::select_rows(&inputs(), &BTreeMap::new(), GuardrailInputType::Request).unwrap();
    assert!(selected.is_none());
}

#[tokio::test]
async fn apply_guardrail_returns_the_inputs_when_there_is_nothing_to_check() {
    let guardrail = build(&args());
    let original = inputs();
    let returned = guardrail
        .apply_guardrail(
            original.clone(),
            &BTreeMap::new(),
            GuardrailInputType::Request,
        )
        .await
        .unwrap();
    assert_eq!(returned, original);
}

#[test]
fn evaluate_payload_carries_the_enabled_checks_and_assertions() {
    let mut args = args();
    args.pii_check = Some(true);
    args.content_moderation_check = Some(true);
    args.assertions = Some(vec!["be brief".to_owned()]);
    let guardrail = build(&args);
    let assertions = ["be brief".to_owned()];
    let check = CheckRequest {
        rows: &[row("hi")],
        output: Some("answer"),
        evaluation_id: None,
        assertions: &assertions,
        available_tools: None,
    };
    let plan = guardrail.plan(&check).unwrap();
    assert_eq!(
        plan.url,
        "https://proxy.qualifire.ai/api/evaluation/evaluate"
    );
    let sent = payload_value(&plan.payload());
    assert_eq!(
        sent,
        json(
            r#"{"messages": [{"role": "user", "content": "hi"}], "output": "answer", "pii_check": true, "content_moderation_check": true, "assertions": ["be brief"]}"#
        )
    );
}

#[test]
fn evaluate_payload_skips_tool_selection_without_tools() {
    let mut args = args();
    args.tool_selection_quality_check = Some(true);
    let guardrail = build(&args);
    let check = CheckRequest {
        rows: &[row("hi")],
        output: None,
        evaluation_id: None,
        assertions: &[],
        available_tools: Some(&json("[]")),
    };
    let plan = guardrail.plan(&check).unwrap();
    assert_eq!(
        payload_value(&plan.payload()),
        json(r#"{"messages": [{"role": "user", "content": "hi"}]}"#)
    );
}

#[test]
fn evaluate_payload_sends_available_tools_with_the_tool_check() {
    let mut args = args();
    args.tool_selection_quality_check = Some(true);
    let guardrail = build(&args);
    let tools = json(r#"[{"type": "function", "function": {"name": "lookup"}}]"#);
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
        json(
            r#"{"messages": [{"role": "user", "content": "hi"}], "tool_selection_quality_check": true, "available_tools": [{"name": "lookup", "description": "", "parameters": {}}]}"#
        )
    );
}

#[test]
fn invoke_payload_takes_the_last_user_content_and_tools() {
    let guardrail = build(&args());
    let tools = json(r#"[{"name": "lookup"}]"#);
    let rows = vec![
        row("first"),
        BTreeMap::from([
            ("role".to_owned(), text("user")),
            ("content".to_owned(), text("second")),
        ]),
    ];
    let check = CheckRequest {
        rows: &rows,
        output: Some("answer"),
        evaluation_id: Some("e1"),
        assertions: &[],
        available_tools: Some(&tools),
    };
    let plan = guardrail.plan(&check).unwrap();
    assert_eq!(plan.url, "https://proxy.qualifire.ai/api/evaluation/invoke");
    assert_eq!(
        payload_value(&plan.payload()),
        json(
            r#"{"evaluation_id": "e1", "input": "second", "output": "answer", "messages": [{"role": "user", "content": "first"}, {"role": "user", "content": "second"}], "available_tools": [{"name": "lookup", "description": "", "parameters": {}}]}"#
        )
    );
}

#[test]
fn invoke_payload_sends_an_empty_output_without_one() {
    let guardrail = build(&args());
    let check = CheckRequest {
        rows: &[row("hi")],
        output: None,
        evaluation_id: Some("e1"),
        assertions: &[],
        available_tools: None,
    };
    let plan = guardrail.plan(&check).unwrap();
    assert_eq!(
        payload_value(&plan.payload()),
        json(
            r#"{"evaluation_id": "e1", "input": "hi", "output": "", "messages": [{"role": "user", "content": "hi"}]}"#
        )
    );
}

#[test]
fn verdict_flags_a_true_flagged_entry() {
    let verdict: Verdict = serde_json::from_str(
        r#"{"evaluationResults": [{"results": [{"flagged": false}, {"flagged": true}]}], "score": 10, "status": "failed"}"#,
    )
    .unwrap();
    assert!(verdict.is_flagged());
    assert_eq!(
        verdict.response(),
        QualifireResponse {
            score: json("10"),
            status: text("failed")
        }
    );
}

#[test]
fn verdict_ignores_false_missing_and_other_flags() {
    let verdict: Verdict = serde_json::from_str(
        r#"{"evaluationResults": [{"results": [{"flagged": false}, {}, {"flagged": "yes"}]}]}"#,
    )
    .unwrap();
    assert!(!verdict.is_flagged());
}

#[test]
fn verdict_reports_absent_score_and_status_as_null() {
    let verdict: Verdict = serde_json::from_str("{}").unwrap();
    assert_eq!(
        verdict.response(),
        QualifireResponse {
            score: GuardrailValue::Null,
            status: GuardrailValue::Null
        }
    );
}

#[test]
fn block_detail_carries_betterlites_error_and_the_qualifire_response() {
    let detail = BlockDetail {
        error: "Violated guardrail policy".to_owned(),
        qualifire_response: QualifireResponse {
            score: json("5"),
            status: text("failed"),
        },
    };
    assert_eq!(
        json(&serde_json::to_string(&detail).unwrap()),
        json(
            r#"{"error": "Violated guardrail policy", "qualifire_response": {"score": 5, "status": "failed"}}"#
        )
    );
}

#[test]
fn api_message_serializes_only_the_set_optional_fields() {
    let converted = ApiMessage {
        role: "user".to_owned(),
        content: "hi".to_owned(),
        tool_call_id: None,
        tool_calls: None,
    };
    assert_eq!(
        json(&serde_json::to_string(&converted).unwrap()),
        json(r#"{"role": "user", "content": "hi"}"#)
    );
}

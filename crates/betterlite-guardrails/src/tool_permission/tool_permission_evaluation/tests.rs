#![cfg(test)]

use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use rstest::rstest;
use tracing::subscriber::with_default;

use super::super::ToolPermissionGuardrail;
use super::super::matching::CompiledRules;
use super::super::rule::{ToolDecision, ToolPermissionRule};
use super::get_permission_for_tool_call;
use crate::guardrail_initializers::tool_permission::{
    ToolDefaultAction, ToolDisallowedAction, ToolPermissionArgs,
};

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl io::Write for Captured {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("poisoned"))?
            .extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn captured_logs<T, F>(level: tracing::Level, action: F) -> (T, String)
where
    F: FnOnce() -> T,
{
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(level)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    let value = with_default(subscriber, action);
    let output = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    (value, output)
}

fn args(default_action: ToolDefaultAction) -> ToolPermissionArgs {
    ToolPermissionArgs {
        name: "tools-check".to_owned(),
        event_hook: LitellmParamsMode::One("pre_call".to_owned()),
        rules: None,
        default_action,
        on_disallowed_action: ToolDisallowedAction::Block,
        default_on: false,
        violation_message_template: None,
    }
}

fn build(args: ToolPermissionArgs, rules: Vec<ToolPermissionRule>) -> ToolPermissionGuardrail {
    let mut compiled = CompiledRules::default();
    for rule in &rules {
        compiled.compile_rule(rule).expect("the test rule compiles");
    }
    ToolPermissionGuardrail {
        args,
        rules,
        compiled,
    }
}

fn pattern_map(patterns: &[(&str, &str)]) -> BTreeMap<String, String> {
    patterns
        .iter()
        .map(|(path, pattern)| ((*path).to_owned(), (*pattern).to_owned()))
        .collect()
}

fn named_rule(id: &str, decision: ToolDecision, tool_name: &str) -> ToolPermissionRule {
    ToolPermissionRule {
        id: id.to_owned(),
        tool_name: Some(tool_name.to_owned()),
        tool_type: None,
        decision,
        allowed_param_patterns: None,
    }
}

fn pattern_rule(
    id: &str,
    decision: ToolDecision,
    tool_name: &str,
    patterns: &[(&str, &str)],
) -> ToolPermissionRule {
    ToolPermissionRule {
        id: id.to_owned(),
        tool_name: Some(tool_name.to_owned()),
        tool_type: None,
        decision,
        allowed_param_patterns: Some(pattern_map(patterns)),
    }
}

fn type_pattern_rule(
    id: &str,
    decision: ToolDecision,
    tool_type: &str,
    patterns: &[(&str, &str)],
) -> ToolPermissionRule {
    ToolPermissionRule {
        id: id.to_owned(),
        tool_name: None,
        tool_type: Some(tool_type.to_owned()),
        decision,
        allowed_param_patterns: Some(pattern_map(patterns)),
    }
}

fn json_arguments(json: &str) -> GuardrailValue {
    serde_json::from_str(json).expect("valid JSON arguments")
}

fn deeply_nested_object(nesting: usize, leaf: &str) -> GuardrailValue {
    let mut value = GuardrailValue::Object(BTreeMap::from([(
        "leaf".to_owned(),
        GuardrailValue::Text(leaf.to_owned()),
    )]));
    for _ in 0..nesting {
        value = GuardrailValue::Object(BTreeMap::from([("a".to_owned(), value)]));
    }
    value
}

#[rstest]
#[case::default_allow(ToolDefaultAction::Allow, true)]
#[case::default_deny(ToolDefaultAction::Deny, false)]
fn a_tool_without_a_name_or_type_short_circuits_to_the_default_action(
    #[case] default_action: ToolDefaultAction,
    #[case] expected_allowed: bool,
) {
    let mut parameters = args(default_action);
    parameters.violation_message_template = Some("rendered: {default_message}".to_owned());
    let guardrail = build(
        parameters,
        vec![named_rule("deny-any", ToolDecision::Deny, ".*")],
    );
    let result = get_permission_for_tool_call(&guardrail, None, None, None);
    assert_eq!(result.is_allowed, expected_allowed);
    assert_eq!(result.rule_id, None);
    assert_eq!(result.message, None);
}

#[test]
fn an_empty_tool_name_is_used_as_given() {
    let guardrail = build(
        args(ToolDefaultAction::Deny),
        vec![named_rule("empty-name", ToolDecision::Allow, "^$")],
    );
    let result = get_permission_for_tool_call(&guardrail, Some(""), Some("function"), None);
    assert!(result.is_allowed);
    assert_eq!(result.rule_id.as_deref(), Some("empty-name"));
}

#[rstest]
#[case::allow_rule(ToolDecision::Allow, true, "allowed")]
#[case::deny_rule(ToolDecision::Deny, false, "denied")]
fn a_matching_rule_decides_the_tool(
    #[case] decision: ToolDecision,
    #[case] expected_allowed: bool,
    #[case] verdict: &str,
) {
    let guardrail = build(
        args(ToolDefaultAction::Deny),
        vec![named_rule("read", decision, "^read_file$")],
    );
    let result =
        get_permission_for_tool_call(&guardrail, Some("read_file"), Some("function"), None);
    assert_eq!(result.is_allowed, expected_allowed);
    assert_eq!(result.rule_id.as_deref(), Some("read"));
    let expected = format!("Tool 'read_file' {verdict} by rule 'read'");
    assert_eq!(result.message.as_deref(), Some(expected.as_str()));
}

#[rstest]
#[case::default_allow(ToolDefaultAction::Allow, true, "allowed")]
#[case::default_deny(ToolDefaultAction::Deny, false, "denied")]
fn a_tool_without_a_matching_rule_uses_the_default_action(
    #[case] default_action: ToolDefaultAction,
    #[case] expected_allowed: bool,
    #[case] verdict: &str,
) {
    let guardrail = build(
        args(default_action),
        vec![named_rule("write", ToolDecision::Allow, "^write$")],
    );
    let result = get_permission_for_tool_call(&guardrail, Some("read"), Some("function"), None);
    assert_eq!(result.is_allowed, expected_allowed);
    assert_eq!(result.rule_id, None);
    let expected = format!("Tool 'read' {verdict} by default action");
    assert_eq!(result.message.as_deref(), Some(expected.as_str()));
}

#[test]
fn the_first_matching_rule_wins() {
    let guardrail = build(
        args(ToolDefaultAction::Deny),
        vec![
            named_rule("allow-read", ToolDecision::Allow, "^read$"),
            named_rule("deny-read", ToolDecision::Deny, "^read$"),
        ],
    );
    let result = get_permission_for_tool_call(&guardrail, Some("read"), Some("function"), None);
    assert!(result.is_allowed);
    assert_eq!(result.rule_id.as_deref(), Some("allow-read"));
}

#[test]
fn a_rule_with_a_name_and_a_type_requires_both_to_match() {
    let rule = ToolPermissionRule {
        id: "both".to_owned(),
        tool_name: Some("^read$".to_owned()),
        tool_type: Some("^function$".to_owned()),
        decision: ToolDecision::Deny,
        allowed_param_patterns: None,
    };
    let guardrail = build(args(ToolDefaultAction::Allow), vec![rule]);
    let result = get_permission_for_tool_call(&guardrail, Some("read"), Some("mcp"), None);
    assert!(result.is_allowed);
    assert_eq!(result.rule_id, None);
}

#[rstest]
#[case::type_matches("function", false, Some("typed"))]
#[case::type_mismatch("mcp", true, None)]
fn a_type_only_rule_matches_on_the_tool_type(
    #[case] tool_type: &str,
    #[case] expected_allowed: bool,
    #[case] expected_rule: Option<&str>,
) {
    let rule = type_pattern_rule(
        "typed",
        ToolDecision::Deny,
        "^function$",
        &[("path", "^/safe$")],
    );
    let guardrail = build(args(ToolDefaultAction::Allow), vec![rule]);
    let result = get_permission_for_tool_call(&guardrail, Some("read"), Some(tool_type), None);
    assert_eq!(result.is_allowed, expected_allowed);
    assert_eq!(result.rule_id.as_deref(), expected_rule);
}

#[test]
fn a_tool_with_a_name_but_no_type_still_evaluates_rules() {
    let guardrail = build(
        args(ToolDefaultAction::Deny),
        vec![named_rule("read", ToolDecision::Allow, "^read$")],
    );
    let result = get_permission_for_tool_call(&guardrail, Some("read"), None, None);
    assert!(result.is_allowed);
    assert_eq!(result.rule_id.as_deref(), Some("read"));
}

#[test]
fn a_tool_with_a_type_but_no_name_still_evaluates_rules() {
    let guardrail = build(
        args(ToolDefaultAction::Deny),
        vec![type_pattern_rule(
            "typed",
            ToolDecision::Allow,
            "^function$",
            &[],
        )],
    );
    let result = get_permission_for_tool_call(&guardrail, None, Some("function"), None);
    assert!(result.is_allowed);
    assert_eq!(result.rule_id.as_deref(), Some("typed"));
    assert_eq!(
        result.message.as_deref(),
        Some("Tool 'function' allowed by rule 'typed'")
    );
}

#[test]
fn nested_dot_and_array_paths_are_collected() {
    let guardrail = build(
        args(ToolDefaultAction::Deny),
        vec![pattern_rule(
            "nested",
            ToolDecision::Allow,
            "^read$",
            &[
                ("user.role", "^admin$"),
                ("files[]", "^(a|b)$"),
                ("matrix[][]", "^[0-9]$"),
            ],
        )],
    );
    let arguments =
        json_arguments(r#"{"user": {"role": "admin"}, "files": ["a", "b"], "matrix": [[1, 2]]}"#);
    let result =
        get_permission_for_tool_call(&guardrail, Some("read"), Some("function"), Some(&arguments));
    assert!(result.is_allowed);
    assert_eq!(result.rule_id.as_deref(), Some("nested"));
}

#[test]
fn a_mismatched_array_item_fails_the_rule() {
    let guardrail = build(
        args(ToolDefaultAction::Deny),
        vec![pattern_rule(
            "only-a",
            ToolDecision::Allow,
            "^read$",
            &[("files[]", "^a$")],
        )],
    );
    let arguments = json_arguments(r#"{"files": ["a", "b"]}"#);
    let result =
        get_permission_for_tool_call(&guardrail, Some("read"), Some("function"), Some(&arguments));
    assert!(!result.is_allowed);
    assert_eq!(result.rule_id, None);
    assert_eq!(
        result.message.as_deref(),
        Some("Value 'b' for path 'files[]' does not match allowed pattern '^a$' for tool 'read'")
    );
}

#[test]
fn a_missing_path_fails_the_rule_and_the_next_rule_decides() {
    let guardrail = build(
        args(ToolDefaultAction::Deny),
        vec![
            pattern_rule(
                "needs-path",
                ToolDecision::Allow,
                "^read$",
                &[("user.role", "^admin$")],
            ),
            named_rule("deny-read", ToolDecision::Deny, "^read$"),
        ],
    );
    let arguments = json_arguments(r#"{"user": {"name": "sam"}}"#);
    let result =
        get_permission_for_tool_call(&guardrail, Some("read"), Some("function"), Some(&arguments));
    assert!(!result.is_allowed);
    assert_eq!(result.rule_id.as_deref(), Some("deny-read"));
}

#[test]
fn the_last_pattern_failure_message_is_kept_when_denying_by_default() {
    let guardrail = build(
        args(ToolDefaultAction::Deny),
        vec![
            pattern_rule("first", ToolDecision::Allow, "^read$", &[("a", "^x$")]),
            pattern_rule("second", ToolDecision::Allow, "^read$", &[("b", "^y$")]),
        ],
    );
    let arguments = json_arguments(r#"{"a": "1", "b": "2"}"#);
    let result =
        get_permission_for_tool_call(&guardrail, Some("read"), Some("function"), Some(&arguments));
    assert!(!result.is_allowed);
    assert_eq!(result.rule_id, None);
    assert_eq!(
        result.message.as_deref(),
        Some("Value '2' for path 'b' does not match allowed pattern '^y$' for tool 'read'")
    );
}

#[test]
fn a_missing_path_message_is_kept_when_denying_by_default() {
    let guardrail = build(
        args(ToolDefaultAction::Deny),
        vec![pattern_rule(
            "needs-path",
            ToolDecision::Allow,
            "^read$",
            &[("user.role", "^admin$")],
        )],
    );
    let arguments = json_arguments(r#"{"user": {"name": "sam"}}"#);
    let result =
        get_permission_for_tool_call(&guardrail, Some("read"), Some("function"), Some(&arguments));
    assert!(!result.is_allowed);
    assert_eq!(result.rule_id, None);
    assert_eq!(
        result.message.as_deref(),
        Some("Missing value for path 'user.role' required by rule 'needs-path'")
    );
}

#[test]
fn a_default_allow_drops_the_pattern_failure_message() {
    let guardrail = build(
        args(ToolDefaultAction::Allow),
        vec![pattern_rule(
            "first",
            ToolDecision::Allow,
            "^read$",
            &[("a", "^x$")],
        )],
    );
    let arguments = json_arguments(r#"{"a": "1"}"#);
    let result =
        get_permission_for_tool_call(&guardrail, Some("read"), Some("function"), Some(&arguments));
    assert!(result.is_allowed);
    assert_eq!(result.rule_id, None);
    assert_eq!(
        result.message.as_deref(),
        Some("Tool 'read' allowed by default action")
    );
}

#[test]
fn a_rule_with_patterns_skips_a_tool_without_arguments() {
    let guardrail = build(
        args(ToolDefaultAction::Deny),
        vec![
            pattern_rule(
                "needs-args",
                ToolDecision::Allow,
                "^read$",
                &[("path", "^/safe$")],
            ),
            named_rule("deny-read", ToolDecision::Deny, "^read$"),
        ],
    );
    let result = get_permission_for_tool_call(&guardrail, Some("read"), Some("function"), None);
    assert!(!result.is_allowed);
    assert_eq!(result.rule_id.as_deref(), Some("deny-read"));
}

#[test]
fn a_missing_argument_failure_is_not_the_default_message() {
    let guardrail = build(
        args(ToolDefaultAction::Deny),
        vec![pattern_rule(
            "needs-args",
            ToolDecision::Allow,
            "^read$",
            &[("path", "^/safe$")],
        )],
    );
    let result = get_permission_for_tool_call(&guardrail, Some("read"), Some("function"), None);
    assert!(!result.is_allowed);
    assert_eq!(result.rule_id, None);
    assert_eq!(
        result.message.as_deref(),
        Some("Tool 'read' denied by default action")
    );
}

#[test]
fn malformed_json_arguments_log_a_warning_and_skip_the_rule() {
    let guardrail = build(
        args(ToolDefaultAction::Deny),
        vec![
            pattern_rule(
                "needs-args",
                ToolDecision::Allow,
                "^read$",
                &[("path", "^/safe$")],
            ),
            named_rule("deny-read", ToolDecision::Deny, "^read$"),
        ],
    );
    let arguments = GuardrailValue::Text("{not json".to_owned());
    let (result, output) = captured_logs(tracing::Level::WARN, || {
        get_permission_for_tool_call(&guardrail, Some("read"), Some("function"), Some(&arguments))
    });
    assert!(!result.is_allowed);
    assert_eq!(result.rule_id.as_deref(), Some("deny-read"));
    assert!(
        output.contains("Failed to decode arguments for tool read"),
        "unexpected log output {output}"
    );
}

#[test]
fn non_object_json_arguments_log_a_debug_rejection() {
    let guardrail = build(
        args(ToolDefaultAction::Deny),
        vec![pattern_rule(
            "needs-args",
            ToolDecision::Allow,
            "^read$",
            &[("path", "^/safe$")],
        )],
    );
    let arguments = GuardrailValue::Text("[1, 2]".to_owned());
    let (result, output) = captured_logs(tracing::Level::DEBUG, || {
        get_permission_for_tool_call(&guardrail, Some("read"), Some("function"), Some(&arguments))
    });
    assert!(!result.is_allowed);
    assert_eq!(result.rule_id, None);
    assert!(
        output.contains("Rejecting non-dict arguments for tool read"),
        "unexpected log output {output}"
    );
}

#[test]
fn arguments_are_not_parsed_for_a_rule_without_patterns() {
    let guardrail = build(
        args(ToolDefaultAction::Deny),
        vec![named_rule("allow-read", ToolDecision::Allow, "^read$")],
    );
    let arguments = GuardrailValue::Text("{not json".to_owned());
    let (result, output) = captured_logs(tracing::Level::WARN, || {
        get_permission_for_tool_call(&guardrail, Some("read"), Some("function"), Some(&arguments))
    });
    assert!(result.is_allowed);
    assert!(output.is_empty(), "unexpected log output {output}");
}

#[rstest]
#[case::json_array(GuardrailValue::Text("[1, 2]".to_owned()))]
#[case::json_string(GuardrailValue::Text("\"text\"".to_owned()))]
#[case::boolean(GuardrailValue::Boolean(true))]
#[case::null(GuardrailValue::Null)]
fn arguments_that_are_not_an_object_skip_the_rule(#[case] arguments: GuardrailValue) {
    let guardrail = build(
        args(ToolDefaultAction::Deny),
        vec![
            pattern_rule(
                "needs-args",
                ToolDecision::Allow,
                "^read$",
                &[("path", "^/safe$")],
            ),
            named_rule("deny-read", ToolDecision::Deny, "^read$"),
        ],
    );
    let result =
        get_permission_for_tool_call(&guardrail, Some("read"), Some("function"), Some(&arguments));
    assert!(!result.is_allowed);
    assert_eq!(result.rule_id.as_deref(), Some("deny-read"));
}

#[rstest]
#[case::absent(None)]
#[case::empty_text(Some(GuardrailValue::Text(String::new())))]
#[case::empty_object(Some(GuardrailValue::Object(BTreeMap::new())))]
#[case::json_empty_object(Some(GuardrailValue::Text("{}".to_owned())))]
fn missing_arguments_skip_a_rule_that_requires_patterns(#[case] arguments: Option<GuardrailValue>) {
    let guardrail = build(
        args(ToolDefaultAction::Deny),
        vec![
            pattern_rule(
                "needs-args",
                ToolDecision::Allow,
                "^read$",
                &[("path", "^/safe$")],
            ),
            named_rule("deny-read", ToolDecision::Deny, "^read$"),
        ],
    );
    let result = get_permission_for_tool_call(
        &guardrail,
        Some("read"),
        Some("function"),
        arguments.as_ref(),
    );
    assert!(!result.is_allowed);
    assert_eq!(result.rule_id.as_deref(), Some("deny-read"));
}

#[test]
fn empty_argument_text_is_missing_not_malformed() {
    let guardrail = build(
        args(ToolDefaultAction::Deny),
        vec![
            pattern_rule(
                "needs-args",
                ToolDecision::Allow,
                "^read$",
                &[("path", "^/safe$")],
            ),
            named_rule("deny-read", ToolDecision::Deny, "^read$"),
        ],
    );
    let arguments = GuardrailValue::Text(String::new());
    let (result, output) = captured_logs(tracing::Level::WARN, || {
        get_permission_for_tool_call(&guardrail, Some("read"), Some("function"), Some(&arguments))
    });
    assert!(!result.is_allowed);
    assert_eq!(result.rule_id.as_deref(), Some("deny-read"));
    assert!(output.is_empty(), "unexpected log output {output}");
}

#[test]
fn an_object_argument_is_used_as_given() {
    let guardrail = build(
        args(ToolDefaultAction::Deny),
        vec![pattern_rule(
            "safe",
            ToolDecision::Allow,
            "^read$",
            &[("path", "^/safe$")],
        )],
    );
    let arguments = GuardrailValue::Object(BTreeMap::from([(
        "path".to_owned(),
        GuardrailValue::Text("/safe".to_owned()),
    )]));
    let result =
        get_permission_for_tool_call(&guardrail, Some("read"), Some("function"), Some(&arguments));
    assert!(result.is_allowed);
    assert_eq!(result.rule_id.as_deref(), Some("safe"));
}

#[test]
fn arguments_parse_from_the_json_string_form() {
    let guardrail = build(
        args(ToolDefaultAction::Deny),
        vec![pattern_rule(
            "safe",
            ToolDecision::Allow,
            "^read$",
            &[("path", "^/safe$")],
        )],
    );
    let arguments = GuardrailValue::Text(r#"{"path": "/safe"}"#.to_owned());
    let result =
        get_permission_for_tool_call(&guardrail, Some("read"), Some("function"), Some(&arguments));
    assert!(result.is_allowed);
    assert_eq!(result.rule_id.as_deref(), Some("safe"));
}

#[test]
fn an_empty_pattern_map_imposes_no_argument_constraint() {
    let rule = ToolPermissionRule {
        id: "empty".to_owned(),
        tool_name: Some("^read$".to_owned()),
        tool_type: None,
        decision: ToolDecision::Allow,
        allowed_param_patterns: Some(BTreeMap::new()),
    };
    let guardrail = build(args(ToolDefaultAction::Deny), vec![rule]);
    let result = get_permission_for_tool_call(&guardrail, Some("read"), Some("function"), None);
    assert!(result.is_allowed);
    assert_eq!(result.rule_id.as_deref(), Some("empty"));
}

#[test]
fn a_duplicate_rule_id_uses_the_compiled_patterns_of_that_id() {
    let guardrail = build(
        args(ToolDefaultAction::Deny),
        vec![
            pattern_rule("dup", ToolDecision::Allow, "^read$", &[("path", "^/safe$")]),
            pattern_rule("dup", ToolDecision::Deny, "^read$", &[("path", "^/other$")]),
        ],
    );
    let arguments = json_arguments(r#"{"path": "/other"}"#);
    let result =
        get_permission_for_tool_call(&guardrail, Some("read"), Some("function"), Some(&arguments));
    // LiteLLM compiles patterns by rule id, so the first rule runs against the second rule's pattern
    assert!(result.is_allowed);
    assert_eq!(result.rule_id.as_deref(), Some("dup"));
}

#[rstest]
#[case::string(r#"{"name": "sam"}"#, "name", "^sam$")]
#[case::integer(r#"{"count": 3}"#, "count", "^3$")]
#[case::decimal(r#"{"ratio": 1.5}"#, "ratio", r"^1\.5$")]
#[case::whole_decimal(r#"{"ratio": 1.0}"#, "ratio", r"^1\.0$")]
#[case::boolean(r#"{"flag": true}"#, "flag", "^true$")]
#[case::null(r#"{"value": null}"#, "value", "^null$")]
fn scalar_argument_values_match_their_json_text(
    #[case] json: &str,
    #[case] path: &str,
    #[case] pattern: &str,
) {
    let rule = pattern_rule("scalar", ToolDecision::Allow, "^read$", &[(path, pattern)]);
    let guardrail = build(args(ToolDefaultAction::Deny), vec![rule]);
    let arguments = json_arguments(json);
    let result =
        get_permission_for_tool_call(&guardrail, Some("read"), Some("function"), Some(&arguments));
    assert!(result.is_allowed, "unexpected denial: {:?}", result.message);
}

#[test]
fn a_non_finite_decimal_argument_matches_as_json_null() {
    let rule = pattern_rule(
        "finite",
        ToolDecision::Allow,
        "^read$",
        &[("value", "^null$")],
    );
    let guardrail = build(args(ToolDefaultAction::Deny), vec![rule]);
    let arguments = GuardrailValue::Object(BTreeMap::from([(
        "value".to_owned(),
        GuardrailValue::Decimal(f64::NAN),
    )]));
    let result =
        get_permission_for_tool_call(&guardrail, Some("read"), Some("function"), Some(&arguments));
    assert!(result.is_allowed, "unexpected denial: {:?}", result.message);
    assert_eq!(result.rule_id.as_deref(), Some("finite"));
}

#[rstest]
#[case::at_the_limit(99, true)]
#[case::past_the_limit(100, false)]
fn nested_argument_paths_stop_at_the_recursion_limit(
    #[case] nesting: usize,
    #[case] expected_allowed: bool,
) {
    let path = format!("{}leaf", "a.".repeat(nesting));
    let rule = ToolPermissionRule {
        id: "deep".to_owned(),
        tool_name: Some("^read$".to_owned()),
        tool_type: None,
        decision: ToolDecision::Allow,
        allowed_param_patterns: Some(BTreeMap::from([(path, "^x$".to_owned())])),
    };
    let guardrail = build(args(ToolDefaultAction::Deny), vec![rule]);
    let arguments = deeply_nested_object(nesting, "x");
    let result =
        get_permission_for_tool_call(&guardrail, Some("read"), Some("function"), Some(&arguments));
    assert_eq!(result.is_allowed, expected_allowed);
    assert_eq!(
        result.rule_id.as_deref(),
        expected_allowed.then_some("deep")
    );
}

#[test]
fn a_rule_decision_renders_the_violation_message_template() {
    let mut parameters = args(ToolDefaultAction::Deny);
    parameters.violation_message_template =
        Some("blocked {tool_name} by {rule_id}: {default_message}".to_owned());
    let guardrail = build(
        parameters,
        vec![named_rule("deny-bash", ToolDecision::Deny, "^Bash$")],
    );
    let result = get_permission_for_tool_call(&guardrail, Some("Bash"), Some("function"), None);
    assert_eq!(
        result.message.as_deref(),
        Some("blocked Bash by deny-bash: Tool 'Bash' denied by rule 'deny-bash'")
    );
}

#[test]
fn a_default_decision_renders_the_violation_message_template() {
    let mut parameters = args(ToolDefaultAction::Deny);
    parameters.violation_message_template = Some("blocked {tool_name}".to_owned());
    let guardrail = build(parameters, Vec::new());
    let result = get_permission_for_tool_call(&guardrail, Some("Bash"), Some("function"), None);
    assert_eq!(result.message.as_deref(), Some("blocked Bash"));
}

#[test]
fn the_first_failing_path_in_sorted_order_is_reported() {
    let guardrail = build(
        args(ToolDefaultAction::Deny),
        vec![pattern_rule(
            "both-paths",
            ToolDecision::Allow,
            "^read$",
            &[("b", "^y$"), ("a", "^x$")],
        )],
    );
    let arguments = json_arguments(r#"{"a": "1", "b": "2"}"#);
    let result =
        get_permission_for_tool_call(&guardrail, Some("read"), Some("function"), Some(&arguments));
    assert!(!result.is_allowed);
    assert_eq!(
        result.message.as_deref(),
        Some("Value '1' for path 'a' does not match allowed pattern '^x$' for tool 'read'")
    );
}

#[test]
fn the_fallback_template_without_a_rule_id_uses_the_default_message() {
    let mut parameters = args(ToolDefaultAction::Deny);
    parameters.violation_message_template = Some("blocked {rule_id}".to_owned());
    let guardrail = build(parameters, Vec::new());
    let (result, output) = captured_logs(tracing::Level::WARN, || {
        get_permission_for_tool_call(&guardrail, Some("Bash"), Some("function"), None)
    });
    assert!(!result.is_allowed);
    assert_eq!(result.rule_id, None);
    assert_eq!(
        result.message.as_deref(),
        Some("Tool 'Bash' denied by default action")
    );
    assert!(
        output.contains("Failed to format violation message template"),
        "unexpected log output {output}"
    );
}

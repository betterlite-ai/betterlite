#![cfg(test)]

use betterlite_config::sections::guardrail_params::base::UnreachableFallback;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use rstest::rstest;

use super::initialize_conduct;
use crate::third_party_init_a::error::ThirdPartyInitAError;
use crate::third_party_init_a::tests::{exposed, params};

#[test]
fn conduct_takes_the_defaults_of_an_empty_config() {
    let built = initialize_conduct(&params(""), None).unwrap();
    assert_eq!(built.name, "");
    assert_eq!(built.api_url, None);
    assert!(built.agent_token.is_none());
    assert_eq!(built.workspace_id, None);
    assert_eq!(built.tool_name, "llm_call");
    assert_eq!(built.unreachable_fallback, UnreachableFallback::FailClosed);
    assert_eq!(Some(built.timeout), Some(8.0_f64));
    assert!(!built.default_on);
    assert_eq!(
        built.event_hook,
        LitellmParamsMode::One("pre_call".to_owned())
    );
}

#[test]
fn conduct_copies_the_configured_fields() {
    let built = initialize_conduct(
        &params(
            r#",
            "api_base": "https://conduct.test",
            "api_key": "agent-token",
            "workspace_id": "workspace-1",
            "tool_name": "send_mail",
            "unreachable_fallback": "fail_open",
            "timeout": 2.5,
            "default_on": true
            "#,
        ),
        Some("conduct-guard"),
    )
    .unwrap();
    assert_eq!(built.name, "conduct-guard");
    assert_eq!(built.api_url.as_deref(), Some("https://conduct.test"));
    assert_eq!(exposed(built.agent_token.as_ref()), Some("agent-token"));
    assert_eq!(built.workspace_id.as_deref(), Some("workspace-1"));
    assert_eq!(built.tool_name, "send_mail");
    assert_eq!(built.unreachable_fallback, UnreachableFallback::FailOpen);
    assert_eq!(Some(built.timeout), Some(2.5_f64));
    assert!(built.default_on);
}

#[rstest]
#[case::absent("", 8.0_f64)]
#[case::zero(r#", "timeout": 0"#, 0.0_f64)]
#[case::explicit(r#", "timeout": 30"#, 30.0_f64)]
#[case::empty_text(r#", "timeout": """#, 8.0_f64)]
fn conduct_falls_back_to_eight_seconds_only_when_the_timeout_is_unset(
    #[case] extras: &str,
    #[case] expected: f64,
) {
    let built = initialize_conduct(&params(extras), None).unwrap();
    assert_eq!(Some(built.timeout), Some(expected));
}

#[test]
fn conduct_reads_a_null_tool_name_as_the_default() {
    let built = initialize_conduct(&params(r#", "tool_name": null"#), None).unwrap();
    assert_eq!(built.tool_name, "llm_call");
}

#[test]
fn conduct_keeps_an_empty_tool_name() {
    let built = initialize_conduct(&params(r#", "tool_name": """#), None).unwrap();
    assert_eq!(built.tool_name, "");
}

#[test]
fn conduct_reads_a_null_workspace_id_as_unset() {
    let built = initialize_conduct(&params(r#", "workspace_id": null"#), None).unwrap();
    assert_eq!(built.workspace_id, None);
}

#[rstest]
#[case::workspace_id(r#", "workspace_id": 4"#, "workspace_id")]
#[case::tool_name(r#", "tool_name": ["a"]"#, "tool_name")]
fn conduct_names_the_param_of_a_wrong_type(#[case] extras: &str, #[case] field: &str) {
    let error = initialize_conduct(&params(extras), None).unwrap_err();
    assert!(matches!(
        &error,
        ThirdPartyInitAError::InvalidParam { name } if name == field
    ));
}

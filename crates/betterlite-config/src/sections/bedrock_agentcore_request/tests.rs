#![cfg(test)]

use std::collections::BTreeMap;

use indexmap::IndexMap;
use rstest::rstest;

use super::{ForwardingValue, Settings};
use crate::loader::{ConfigError, ConfigLoader};

fn load(
    params: &str,
    settings: &str,
    environment: &[(&str, &str)],
) -> Result<Settings, ConfigError> {
    let source = format!(
        "model_list:\n- model_name: a\n  litellm_params: {{model: bedrock/agentcore/arn:aws:bedrock-agentcore:us-west-2:1:runtime/r{params}}}\nlitellm_settings: {{{settings}}}\n"
    );
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    let config = ConfigLoader { lookup }.load(&source)?;
    Ok(config.sections.bedrock_agentcore_request)
}

fn enabled(settings: &Settings) -> Option<String> {
    settings
        .forward_multimodal_content
        .as_ref()
        .map(|value| value.is_enabled().to_string())
}

fn session_id(settings: &Settings) -> Option<String> {
    settings.runtime_session_id.clone()
}

fn user_id(settings: &Settings) -> Option<String> {
    settings.runtime_user_id.clone()
}

fn qualifier(settings: &Settings) -> Option<String> {
    settings.qualifier.clone()
}

type Read = fn(&Settings) -> Option<String>;

#[rstest]
#[case::loads_forward_multimodal_content(check_forward_multimodal_content)]
#[case::loads_runtimesessionid(check_runtime_session_id)]
#[case::loads_runtimeuserid(check_runtime_user_id)]
#[case::loads_qualifier(check_qualifier)]
fn agentcore_request_config_cases(#[case] check: fn()) {
    check();
}

fn check_forward_multimodal_content() {
    assert_forwarding_sources();
    assert_forwarding_interpretation();
    assert_unrelated_key_is_reported("forward_multimodal_content");
}

fn assert_forwarding_sources() {
    assert_eq!(enabled(&load("", "", &[]).unwrap()), None, "default");
    assert_eq!(
        enabled(&load("", "forward_multimodal_content: true", &[]).unwrap()),
        Some("true".to_owned()),
        "global"
    );
    assert_eq!(
        enabled(&load(", forward_multimodal_content: false", "", &[]).unwrap()),
        Some("false".to_owned()),
        "deployment"
    );
    assert_eq!(
        enabled(
            &load(
                ", forward_multimodal_content: true",
                "forward_multimodal_content: false",
                &[]
            )
            .unwrap()
        ),
        Some("true".to_owned()),
        "precedence"
    );
    let environment = [("AGENTCORE_FORWARD", "yes")];
    assert_eq!(
        enabled(
            &load(
                "",
                "forward_multimodal_content: os.environ/AGENTCORE_FORWARD",
                &environment
            )
            .unwrap()
        ),
        Some("true".to_owned()),
        "environment"
    );
}

fn assert_forwarding_interpretation() {
    assert_eq!(
        enabled(&load("", "forward_multimodal_content: \"false\"", &[]).unwrap()),
        Some("false".to_owned()),
        "string policy"
    );
    assert_eq!(
        enabled(&load("", "forward_multimodal_content: 0", &[]).unwrap()),
        Some("false".to_owned()),
        "numeric truthiness"
    );
}

fn check_runtime_session_id() {
    check_text_key("runtimeSessionId", "AGENTCORE_SESSION_ID", session_id);
}

fn check_runtime_user_id() {
    check_text_key("runtimeUserId", "AGENTCORE_USER_ID", user_id);
}

fn check_qualifier() {
    check_text_key("qualifier", "AGENTCORE_QUALIFIER", qualifier);
}

fn check_text_key(key: &str, variable: &str, read: Read) {
    assert_eq!(read(&load("", "", &[]).unwrap()), None, "{key} default");
    assert_eq!(
        read(&load("", &format!("{key}: global-value"), &[]).unwrap()),
        Some("global-value".to_owned()),
        "{key} global"
    );
    assert_eq!(
        read(&load(&format!(", {key}: deployment-value"), "", &[]).unwrap()),
        Some("deployment-value".to_owned()),
        "{key} deployment"
    );
    assert_eq!(
        read(
            &load(
                &format!(", {key}: deployment-value"),
                &format!("{key}: global-value"),
                &[]
            )
            .unwrap()
        ),
        Some("deployment-value".to_owned()),
        "{key} precedence"
    );
    assert_eq!(
        read(&load("", &format!("{key}: \"\""), &[]).unwrap()),
        Some(String::new()),
        "{key} explicit empty"
    );
    let environment = [(variable, "from-environment")];
    assert_eq!(
        read(&load("", &format!("{key}: os.environ/{variable}"), &environment).unwrap()),
        Some("from-environment".to_owned()),
        "{key} environment"
    );
    assert_unrelated_key_is_reported(key);
}

fn assert_unrelated_key_is_reported(key: &str) {
    let error = load(&format!(", {key}_unrelated: 1"), "", &[]).unwrap_err();
    assert!(
        matches!(&error, ConfigError::Unsupported { paths } if *paths == [format!("model_list.0.litellm_params.{key}_unrelated")]),
        "{error}"
    );
}

#[test]
fn second_deployment_wins_over_the_first() {
    let source = "model_list:\n- model_name: a\n  litellm_params: {model: bedrock/agentcore/arn:aws:bedrock-agentcore:us-west-2:1:runtime/a, qualifier: first}\n- model_name: b\n  litellm_params: {model: bedrock/agentcore/arn:aws:bedrock-agentcore:us-west-2:1:runtime/b, qualifier: second}\n";
    let config = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .unwrap();
    assert_eq!(
        config.sections.bedrock_agentcore_request.qualifier,
        Some("second".to_owned())
    );
}

#[rstest]
#[case::on_padded(ForwardingValue::Text(" ON ".to_owned()), true)]
#[case::one(ForwardingValue::Text("1".to_owned()), true)]
#[case::true_mixed_case(ForwardingValue::Text("True".to_owned()), true)]
#[case::yes(ForwardingValue::Text("yes".to_owned()), true)]
#[case::off(ForwardingValue::Text("off".to_owned()), false)]
#[case::garbage(ForwardingValue::Text("garbage".to_owned()), false)]
#[case::boolean_true(ForwardingValue::Boolean(true), true)]
#[case::boolean_false(ForwardingValue::Boolean(false), false)]
#[case::integer(ForwardingValue::Integer(2), true)]
#[case::integer_zero(ForwardingValue::Integer(0), false)]
#[case::unsigned(ForwardingValue::Unsigned(u64::MAX), true)]
#[case::unsigned_zero(ForwardingValue::Unsigned(0), false)]
#[case::float(ForwardingValue::Float(0.5), true)]
#[case::float_zero(ForwardingValue::Float(0.0), false)]
#[case::list(ForwardingValue::List(vec![ForwardingValue::Null]), true)]
#[case::empty_list(ForwardingValue::List(Vec::new()), false)]
#[case::mapping(ForwardingValue::Mapping(IndexMap::from([("key".to_owned(), ForwardingValue::Null)])), true)]
#[case::empty_mapping(ForwardingValue::Mapping(IndexMap::new()), false)]
#[case::null(ForwardingValue::Null, false)]
fn forwarding_value_truthiness_matches_litellm(
    #[case] value: ForwardingValue,
    #[case] expected: bool,
) {
    assert_eq!(value.is_enabled(), expected);
}

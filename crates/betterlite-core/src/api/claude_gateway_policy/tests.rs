#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::loader::{ConfigError, ConfigLoader};
use betterlite_config::sections::claude_gateway_policy::ManagedSettings;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use super::{
    ClaudeGatewayPolicyError, ClaudeGatewayPolicyInput, DISABLED_DETAIL, DISABLED_STATUS_CODE,
    ensure_gateway_enabled, evaluate,
};

const ENABLED_TRUE: &str = "general_settings:\n  enable_claude_code_gateway: true\n";
const ENABLED_FALSE: &str = "general_settings:\n  enable_claude_code_gateway: false\n";
const ENABLED_ENV: &str =
    "general_settings:\n  enable_claude_code_gateway: os.environ/CLAUDE_GATEWAY_ENABLED\n";
const MANAGED_DENY_BASH: &str = "general_settings:\n  claude_code_gateway_managed_settings:\n    permissions:\n      deny:\n        - Bash\n";
const DISABLED_WITH_MANAGED: &str = "general_settings:
  enable_claude_code_gateway: false
  claude_code_gateway_managed_settings:
    permissions:
      deny:
        - Bash
";
const SUCCESS: &str = "general_settings:
  enable_claude_code_gateway: true
  claude_code_gateway_managed_settings:
    permissions:
      deny:
        - Bash
    env:
      CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: '1'
";

fn load(yaml: &str, environment: &[(&str, &str)]) -> Result<ClaudeGatewayPolicyInput, ConfigError> {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let config = ConfigLoader { lookup }.load(yaml)?;
    Ok(ClaudeGatewayPolicyInput::from(
        &config.sections.claude_gateway_policy,
    ))
}

fn deny_bash() -> ManagedSettings {
    let mut permissions = IndexMap::new();
    permissions.insert(
        "deny".to_owned(),
        Json::List(vec![Json::Text("Bash".to_owned())]),
    );
    let mut settings = IndexMap::new();
    settings.insert("permissions".to_owned(), Json::Object(permissions));
    settings
}

fn success_settings() -> ManagedSettings {
    let mut env = IndexMap::new();
    env.insert(
        "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC".to_owned(),
        Json::Text("1".to_owned()),
    );
    let mut settings = deny_bash();
    settings.insert("env".to_owned(), Json::Object(env));
    settings
}

fn disabled(error: ClaudeGatewayPolicyError) -> (u16, &'static str) {
    let ClaudeGatewayPolicyError::Disabled {
        status_code,
        detail,
    } = error;
    (status_code, detail)
}

#[test]
fn source_success() {
    let input = load(SUCCESS, &[]).unwrap();
    let result = ensure_gateway_enabled(&input).unwrap();
    assert!(result.enabled);
    assert_eq!(result.managed_settings, Some(&success_settings()));
    assert_eq!(evaluate(&input), result);
}

#[test]
fn source_failure() {
    let input = load(DISABLED_WITH_MANAGED, &[]).unwrap();
    let error = ensure_gateway_enabled(&input).unwrap_err();
    assert_eq!(disabled(error), (404, "Claude Code gateway is not enabled"));
    assert_eq!(DISABLED_STATUS_CODE, 404);
    assert_eq!(DISABLED_DETAIL, "Claude Code gateway is not enabled");
    let state = evaluate(&input);
    assert!(!state.enabled);
    assert_eq!(state.managed_settings, Some(&deny_bash()));
}

#[test]
fn source_defaults() {
    let input = load("{}", &[]).unwrap();
    let state = evaluate(&input);
    assert!(!state.enabled);
    assert_eq!(state.managed_settings, None);
    assert_eq!(evaluate(&ClaudeGatewayPolicyInput::default()), state);
    let error = ensure_gateway_enabled(&ClaudeGatewayPolicyInput::default()).unwrap_err();
    assert_eq!(disabled(error), (404, "Claude Code gateway is not enabled"));
}

type Case = Result<(bool, Option<ManagedSettings>), &'static str>;

#[rstest]
#[case::unset("{}", &[], Ok((false, None)))]
#[case::general_settings_null("general_settings: null\n", &[], Ok((false, None)))]
#[case::enabled_true(ENABLED_TRUE, &[], Ok((true, None)))]
#[case::enabled_false(ENABLED_FALSE, &[], Ok((false, None)))]
#[case::enabled_null("general_settings:\n  enable_claude_code_gateway: ~\n", &[], Ok((false, None)))]
#[case::enabled_environment_on(ENABLED_ENV, &[("CLAUDE_GATEWAY_ENABLED", "yes")], Ok((true, None)))]
#[case::enabled_environment_off(ENABLED_ENV, &[("CLAUDE_GATEWAY_ENABLED", "no")], Ok((false, None)))]
#[case::enabled_environment_empty(ENABLED_ENV, &[("CLAUDE_GATEWAY_ENABLED", "")], Ok((false, None)))]
#[case::enabled_environment_missing(ENABLED_ENV, &[], Err("missing_environment"))]
#[case::enabled_environment_invalid(ENABLED_ENV, &[("CLAUDE_GATEWAY_ENABLED", "perhaps")], Err("environment"))]
#[case::enabled_text("general_settings:\n  enable_claude_code_gateway: enabled\n", &[], Err("invalid_scalar"))]
#[case::enabled_float("general_settings:\n  enable_claude_code_gateway: 1.0\n", &[], Err("invalid_scalar"))]
#[case::managed_configured(MANAGED_DENY_BASH, &[], Ok((false, Some(deny_bash()))))]
#[case::managed_null("general_settings:\n  claude_code_gateway_managed_settings: ~\n", &[], Ok((false, None)))]
#[case::managed_empty("general_settings:\n  claude_code_gateway_managed_settings: {}\n", &[], Ok((false, Some(IndexMap::new()))))]
#[case::managed_text("general_settings:\n  claude_code_gateway_managed_settings: policy\n", &[], Err("invalid_scalar"))]
#[case::managed_nested_list("general_settings:\n  claude_code_gateway_managed_settings:\n    - deny\n", &[], Err("invalid_scalar"))]
fn loads_settings(
    #[case] yaml: &str,
    #[case] environment: &[(&str, &str)],
    #[case] expected: Case,
) {
    match expected {
        Ok((enabled, managed_settings)) => {
            let input = load(yaml, environment).unwrap();
            let state = evaluate(&input);
            assert_eq!(state.enabled, enabled);
            assert_eq!(state.managed_settings, managed_settings.as_ref());
        }
        Err(category) => assert_eq!(load(yaml, environment).unwrap_err().category(), category),
    }
}

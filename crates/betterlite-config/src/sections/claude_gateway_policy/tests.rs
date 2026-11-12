#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::claude_gateway_policy::{ManagedSettings, Settings, recursion_depth};

const ENABLED_TRUE: &str = "general_settings:\n  enable_claude_code_gateway: true\n";
const ENABLED_FALSE: &str = "general_settings:\n  enable_claude_code_gateway: false\n";
const ENABLED_NULL: &str = "general_settings:\n  enable_claude_code_gateway: ~\n";
const ENABLED_ENV: &str =
    "general_settings:\n  enable_claude_code_gateway: os.environ/CLAUDE_GATEWAY_ENABLED\n";
const MANAGED_DENY_BASH: &str = "general_settings:\n  claude_code_gateway_managed_settings:\n    permissions:\n      deny:\n        - Bash\n";
const MANAGED_ENV_URL: &str = "general_settings:\n  claude_code_gateway_managed_settings:\n    env:\n      ANTHROPIC_BASE_URL: os.environ/GATEWAY_URL\n";
const MANAGED_ENV_FLAG: &str = "general_settings:\n  claude_code_gateway_managed_settings:\n    env:\n      CLAUDE_CODE_AUTO_CONNECT_IDE: os.environ/GATEWAY_FLAG\n";
const MANAGED_DENY_REFERENCE: &str = "general_settings:\n  claude_code_gateway_managed_settings:\n    permissions:\n      deny:\n        - os.environ/GATEWAY_DENY\n";
const MANAGED_RULE_COMMAND: &str = "general_settings:\n  claude_code_gateway_managed_settings:\n    rules:\n      - command: os.environ/GATEWAY_COMMAND\n";

fn load(yaml: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let config = ConfigLoader { lookup }.load(yaml)?;
    Ok(config.sections.claude_gateway_policy)
}

fn managed_env(value: Json) -> ManagedSettings {
    let mut env = IndexMap::new();
    env.insert("ANTHROPIC_BASE_URL".to_owned(), value);
    let mut settings = IndexMap::new();
    settings.insert("env".to_owned(), Json::Object(env));
    settings
}

fn managed_flag(value: Json) -> ManagedSettings {
    let mut env = IndexMap::new();
    env.insert("CLAUDE_CODE_AUTO_CONNECT_IDE".to_owned(), value);
    let mut settings = IndexMap::new();
    settings.insert("env".to_owned(), Json::Object(env));
    settings
}

fn managed_deny(value: &str) -> ManagedSettings {
    let mut permissions = IndexMap::new();
    permissions.insert(
        "deny".to_owned(),
        Json::List(vec![Json::Text(value.to_owned())]),
    );
    let mut settings = IndexMap::new();
    settings.insert("permissions".to_owned(), Json::Object(permissions));
    settings
}

fn managed_rule(value: &str) -> ManagedSettings {
    let mut rule = IndexMap::new();
    rule.insert("command".to_owned(), Json::Text(value.to_owned()));
    let mut settings = IndexMap::new();
    settings.insert("rules".to_owned(), Json::List(vec![Json::Object(rule)]));
    settings
}

fn nested_settings_yaml(levels: usize) -> String {
    let mut yaml = String::from("general_settings:\n  claude_code_gateway_managed_settings:\n");
    let mut indent = 4;
    for level in 0..levels {
        yaml.push_str(&" ".repeat(indent));
        yaml.push_str("level");
        yaml.push_str(&level.to_string());
        yaml.push_str(":\n");
        indent = indent.saturating_add(2);
    }
    yaml.push_str(&" ".repeat(indent));
    yaml.push_str("url: os.environ/GATEWAY_URL\n");
    yaml
}

fn nested_expected(levels: usize, leaf: Json) -> ManagedSettings {
    let mut innermost = IndexMap::new();
    innermost.insert("url".to_owned(), leaf);
    let mut current = Json::Object(innermost);
    for level in (0..levels).rev() {
        let mut wrapper = IndexMap::new();
        wrapper.insert(format!("level{level}"), current);
        current = Json::Object(wrapper);
    }
    let Json::Object(settings) = current else {
        panic!("the nested settings are an object");
    };
    settings
}

type Case = Result<(bool, Option<ManagedSettings>), &'static str>;

#[rstest]
#[case::unset("{}", &[], Ok((false, None)))]
#[case::general_settings_null("general_settings: null\n", &[], Ok((false, None)))]
#[case::general_settings_empty("general_settings: {}\n", &[], Ok((false, None)))]
#[case::enabled_true(ENABLED_TRUE, &[], Ok((true, None)))]
#[case::enabled_false(ENABLED_FALSE, &[], Ok((false, None)))]
#[case::enabled_null(ENABLED_NULL, &[], Ok((false, None)))]
#[case::enabled_environment_on(ENABLED_ENV, &[("CLAUDE_GATEWAY_ENABLED", " on ")], Ok((true, None)))]
#[case::enabled_environment_off(ENABLED_ENV, &[("CLAUDE_GATEWAY_ENABLED", "OFF")], Ok((false, None)))]
#[case::enabled_environment_empty(ENABLED_ENV, &[("CLAUDE_GATEWAY_ENABLED", "")], Ok((false, None)))]
#[case::enabled_environment_missing(ENABLED_ENV, &[], Err("missing_environment"))]
#[case::enabled_environment_invalid(ENABLED_ENV, &[("CLAUDE_GATEWAY_ENABLED", "sometimes")], Err("environment"))]
#[case::enabled_text("general_settings:\n  enable_claude_code_gateway: enabled\n", &[], Err("invalid_scalar"))]
#[case::enabled_integer("general_settings:\n  enable_claude_code_gateway: 1\n", &[], Err("invalid_scalar"))]
#[case::managed_configured(MANAGED_DENY_BASH, &[], Ok((false, Some(managed_deny("Bash")))))]
#[case::managed_null("general_settings:\n  claude_code_gateway_managed_settings: ~\n", &[], Ok((false, None)))]
#[case::managed_empty("general_settings:\n  claude_code_gateway_managed_settings: {}\n", &[], Ok((false, Some(IndexMap::new()))))]
#[case::managed_text("general_settings:\n  claude_code_gateway_managed_settings: policy\n", &[], Err("invalid_scalar"))]
#[case::managed_list("general_settings:\n  claude_code_gateway_managed_settings:\n    - deny\n", &[], Err("invalid_scalar"))]
#[case::managed_environment_text(MANAGED_ENV_URL, &[("GATEWAY_URL", "https://gateway.example")], Ok((false, Some(managed_env(Json::Text("https://gateway.example".to_owned()))))))]
#[case::managed_environment_true(MANAGED_ENV_FLAG, &[("GATEWAY_FLAG", " True ")], Ok((false, Some(managed_flag(Json::Bool(true))))))]
#[case::managed_environment_false(MANAGED_ENV_FLAG, &[("GATEWAY_FLAG", "FALSE")], Ok((false, Some(managed_flag(Json::Bool(false))))))]
#[case::managed_environment_other_text(MANAGED_ENV_FLAG, &[("GATEWAY_FLAG", "yes")], Ok((false, Some(managed_flag(Json::Text("yes".to_owned()))))))]
#[case::managed_environment_empty(MANAGED_ENV_FLAG, &[("GATEWAY_FLAG", "")], Ok((false, Some(managed_flag(Json::Text(String::new()))))))]
#[case::managed_environment_unset(MANAGED_ENV_URL, &[], Ok((false, Some(managed_env(Json::Null)))))]
#[case::managed_environment_list_string(MANAGED_DENY_REFERENCE, &[("GATEWAY_DENY", "Bash")], Ok((false, Some(managed_deny("os.environ/GATEWAY_DENY")))))]
#[case::managed_environment_list_object(MANAGED_RULE_COMMAND, &[("GATEWAY_COMMAND", "npm test")], Ok((false, Some(managed_rule("npm test")))))]
#[case::managed_environment_invalid_depth(MANAGED_ENV_URL, &[("DEFAULT_MAX_RECURSE_DEPTH", "many")], Err("environment"))]
fn loads_settings(
    #[case] yaml: &str,
    #[case] environment: &[(&str, &str)],
    #[case] expected: Case,
) {
    match expected {
        Ok((enabled, managed_settings)) => assert_eq!(
            load(yaml, environment).unwrap(),
            Settings {
                enabled,
                managed_settings,
            }
        ),
        Err(category) => assert_eq!(load(yaml, environment).unwrap_err().category(), category),
    }
}

// serde-saphyr stops a YAML document at 64 nesting levels, below LiteLLM's default depth of 100
#[test]
fn resolves_references_through_the_deepest_mapping_yaml_allows() {
    let environment = [("GATEWAY_URL", "https://gateway.example")];
    let settings = load(&nested_settings_yaml(60), &environment).unwrap();
    assert_eq!(
        settings.managed_settings,
        Some(nested_expected(
            60,
            Json::Text("https://gateway.example".to_owned())
        ))
    );
}

#[test]
fn an_unset_depth_variable_keeps_litellms_default() {
    let lookup = BTreeMap::<String, String>::new();
    assert_eq!(recursion_depth(&lookup).unwrap(), 100);
}

#[test]
fn the_recursion_depth_follows_the_environment_variable() {
    let environment = [
        ("DEFAULT_MAX_RECURSE_DEPTH", "3"),
        ("GATEWAY_URL", "https://gateway.example"),
    ];
    let within = load(&nested_settings_yaml(1), &environment).unwrap();
    assert_eq!(
        within.managed_settings,
        Some(nested_expected(
            1,
            Json::Text("https://gateway.example".to_owned())
        ))
    );
    let past = load(&nested_settings_yaml(2), &environment).unwrap();
    assert_eq!(
        past.managed_settings,
        Some(nested_expected(
            2,
            Json::Text("os.environ/GATEWAY_URL".to_owned())
        ))
    );
}

#[test]
fn the_gateway_keys_leave_the_unsupported_report() {
    let yaml = "general_settings:
  enable_claude_code_gateway: true
  claude_code_gateway_managed_settings:
    permissions: {}
  claude_gateway_misspelled: true
";
    let error = load(yaml, &[]).unwrap_err();
    let ConfigError::Unsupported { paths } = error else {
        panic!("the misspelled sibling key must be the only unsupported path");
    };
    assert_eq!(paths, ["general_settings.claude_gateway_misspelled"]);
}

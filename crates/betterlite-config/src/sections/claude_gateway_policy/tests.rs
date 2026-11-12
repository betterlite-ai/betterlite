#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::claude_gateway_policy::{ManagedSettings, Settings};

const ENABLED_TRUE: &str = "general_settings:\n  enable_claude_code_gateway: true\n";
const ENABLED_FALSE: &str = "general_settings:\n  enable_claude_code_gateway: false\n";
const ENABLED_NULL: &str = "general_settings:\n  enable_claude_code_gateway: ~\n";
const ENABLED_ENV: &str =
    "general_settings:\n  enable_claude_code_gateway: os.environ/CLAUDE_GATEWAY_ENABLED\n";
const MANAGED_DENY_BASH: &str = "general_settings:\n  claude_code_gateway_managed_settings:\n    permissions:\n      deny:\n        - Bash\n";

fn load(yaml: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let config = ConfigLoader { lookup }.load(yaml)?;
    Ok(config.sections.claude_gateway_policy)
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
#[case::managed_configured(MANAGED_DENY_BASH, &[], Ok((false, Some(deny_bash()))))]
#[case::managed_null("general_settings:\n  claude_code_gateway_managed_settings: ~\n", &[], Ok((false, None)))]
#[case::managed_empty("general_settings:\n  claude_code_gateway_managed_settings: {}\n", &[], Ok((false, Some(IndexMap::new()))))]
#[case::managed_text("general_settings:\n  claude_code_gateway_managed_settings: policy\n", &[], Err("invalid_scalar"))]
#[case::managed_list("general_settings:\n  claude_code_gateway_managed_settings:\n    - deny\n", &[], Err("invalid_scalar"))]
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

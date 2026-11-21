#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::auth_request_policy::Settings;

fn load(source: &str) -> Result<Settings, ConfigError> {
    Ok(ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(source)?
    .sections
    .auth_request_policy)
}

fn load_with_environment(
    source: &str,
    environment: &[(&str, &str)],
) -> Result<Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .auth_request_policy)
}

#[rstest]
#[case::empty("{}")]
#[case::absent("general_settings:\n  master_key: sk-test\n")]
#[case::explicit_null(
    "general_settings:\n  allowed_ips: null\n  use_x_forwarded_for: null\n  \
     max_request_size_mb: null\n  max_response_size_mb: null\n  allowed_routes: null\n"
)]
fn policy_defaults(#[case] source: &str) {
    assert_eq!(load(source).unwrap(), Settings::default());
}

#[test]
fn exact_ip_and_route_lists() {
    let settings = load(
        "general_settings:\n  allowed_ips: [10.0.0.1, 10.0.0.0/8, '']\n  \
         allowed_routes: ['/v1/chat/completions', '/v1/*']\n",
    )
    .unwrap();
    assert_eq!(
        settings.allowed_ips,
        Some(vec![
            "10.0.0.1".to_owned(),
            "10.0.0.0/8".to_owned(),
            String::new()
        ])
    );
    assert_eq!(
        settings.allowed_routes,
        Some(vec!["/v1/chat/completions".to_owned(), "/v1/*".to_owned()])
    );
    let empty = load("general_settings:\n  allowed_ips: []\n").unwrap();
    assert_eq!(empty.allowed_ips, Some(Vec::new()));
    assert_eq!(empty.allowed_routes, None);
}

#[rstest]
#[case::on("use_x_forwarded_for: true", true)]
#[case::off("use_x_forwarded_for: false", false)]
fn forwarded_header_toggle(#[case] setting: &str, #[case] expected: bool) {
    let source = format!("general_settings:\n  {setting}\n");
    assert_eq!(load(&source).unwrap().use_x_forwarded_for, expected);
}

#[rstest]
#[case::text("use_x_forwarded_for: yes-please")]
#[case::number("use_x_forwarded_for: 1")]
#[case::list("use_x_forwarded_for: [true]")]
fn forwarded_header_rejects_a_non_boolean(#[case] setting: &str) {
    let source = format!("general_settings:\n  {setting}\n");
    assert!(matches!(
        load(&source).unwrap_err(),
        ConfigError::Yaml { .. }
    ));
}

#[rstest]
#[case::fractional(
    "max_request_size_mb: 0.5\n  max_response_size_mb: 0.25",
    0.5_f64,
    0.25_f64
)]
#[case::zero("max_request_size_mb: 0\n  max_response_size_mb: 0", 0.0_f64, 0.0_f64)]
#[case::negative(
    "max_request_size_mb: -2\n  max_response_size_mb: -0.5",
    -2.0_f64,
    -0.5_f64
)]
fn fractional_zero_negative_limits(
    #[case] settings: &str,
    #[case] request: f64,
    #[case] response: f64,
) {
    let source = format!("general_settings:\n  {settings}\n");
    let loaded = load(&source).unwrap();
    assert_eq!(loaded.max_request_size_mb, Some(request));
    assert_eq!(loaded.max_response_size_mb, Some(response));
}

#[rstest]
#[case::request("max_request_size_mb: five")]
#[case::response("max_response_size_mb: true")]
fn limits_reject_a_non_number(#[case] setting: &str) {
    let source = format!("general_settings:\n  {setting}\n");
    assert!(matches!(
        load(&source).unwrap_err(),
        ConfigError::Yaml { .. }
    ));
}

#[test]
fn settings_have_no_root_level_spelling() {
    let error = load("allowed_ips: [10.0.0.1]\n").unwrap_err();
    let ConfigError::Unsupported { paths } = &error else {
        panic!("expected an unsupported key, got {error:?}");
    };
    assert!(paths.iter().any(|path| path == "allowed_ips"), "{paths:?}");
}

#[test]
fn settings_have_no_environment_spelling() {
    let settings = load_with_environment("{}", &[("ALLOWED_IPS", "10.0.0.1")]).unwrap();
    assert_eq!(settings.allowed_ips, None);
}

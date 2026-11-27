use std::collections::BTreeMap;

use secrecy::{ExposeSecret, SecretString};

use crate::watsonx_auth::environment::WatsonxEnvironment;

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

const NAMES: [(&str, &str); 20] = [
    ("WATSONX_IAM_URL", "watsonx-iam-url"),
    ("WX_API_KEY", "wx-api-key"),
    ("WATSONX_API_KEY", "watsonx-api-key"),
    ("WATSONX_APIKEY", "watsonx-apikey"),
    ("WATSONX_ZENAPIKEY", "watsonx-zenapikey"),
    ("WATSONX_TOKEN", "watsonx-token"),
    ("WATSONX_API_BASE", "watsonx-api-base"),
    ("WATSONX_URL", "watsonx-url"),
    ("WX_URL", "wx-url"),
    ("WML_URL", "wml-url"),
    ("WATSONX_PROJECT_ID", "watsonx-project-id"),
    ("WX_PROJECT_ID", "wx-project-id"),
    ("PROJECT_ID", "project-id"),
    ("WATSONX_REGION", "watsonx-region"),
    ("WX_REGION", "wx-region"),
    ("REGION", "region-name"),
    ("WATSONX_DEPLOYMENT_SPACE_ID", "watsonx-deployment-space-id"),
    ("WATSONX_SPACE_ID", "watsonx-space-id"),
    ("WX_SPACE_ID", "wx-space-id"),
    ("SPACE_ID", "space-id"),
];

fn exposed(value: Option<&SecretString>) -> Option<&str> {
    value.map(ExposeSecret::expose_secret)
}

// each variable beside the field that must read it, so a wrong pairing fails the comparison
fn values_by_variable(built: &WatsonxEnvironment) -> Vec<(&'static str, Option<&str>)> {
    vec![
        ("WATSONX_IAM_URL", built.watsonx_iam_url.as_deref()),
        ("WX_API_KEY", exposed(built.wx_api_key.as_ref())),
        ("WATSONX_API_KEY", exposed(built.watsonx_api_key.as_ref())),
        ("WATSONX_APIKEY", exposed(built.watsonx_apikey.as_ref())),
        (
            "WATSONX_ZENAPIKEY",
            exposed(built.watsonx_zenapikey.as_ref()),
        ),
        ("WATSONX_TOKEN", exposed(built.watsonx_token.as_ref())),
        ("WATSONX_API_BASE", built.watsonx_api_base.as_deref()),
        ("WATSONX_URL", built.watsonx_url.as_deref()),
        ("WX_URL", built.wx_url.as_deref()),
        ("WML_URL", built.wml_url.as_deref()),
        ("WATSONX_PROJECT_ID", built.watsonx_project_id.as_deref()),
        ("WX_PROJECT_ID", built.wx_project_id.as_deref()),
        ("PROJECT_ID", built.project_id.as_deref()),
        ("WATSONX_REGION", built.watsonx_region.as_deref()),
        ("WX_REGION", built.wx_region.as_deref()),
        ("REGION", built.region.as_deref()),
        (
            "WATSONX_DEPLOYMENT_SPACE_ID",
            built.watsonx_deployment_space_id.as_deref(),
        ),
        ("WATSONX_SPACE_ID", built.watsonx_space_id.as_deref()),
        ("WX_SPACE_ID", built.wx_space_id.as_deref()),
        ("SPACE_ID", built.space_id.as_deref()),
    ]
}

#[test]
fn from_lookup_fills_each_field_from_its_variable() {
    let built = WatsonxEnvironment::from_lookup(&environment(&NAMES));
    let expected: Vec<(&str, Option<&str>)> = NAMES
        .iter()
        .map(|(name, value)| (*name, Some(*value)))
        .collect();
    assert_eq!(values_by_variable(&built), expected);
}

#[test]
fn from_lookup_leaves_an_empty_variable_unset() {
    let blanked: Vec<(&str, &str)> = NAMES.iter().map(|(name, _value)| (*name, "")).collect();
    let built = WatsonxEnvironment::from_lookup(&environment(&blanked));
    let expected: Vec<(&str, Option<&str>)> =
        NAMES.iter().map(|(name, _value)| (*name, None)).collect();
    assert_eq!(values_by_variable(&built), expected);
}

#[test]
fn from_lookup_leaves_a_missing_variable_unset() {
    let built = WatsonxEnvironment::from_lookup(&environment(&[]));
    let expected: Vec<(&str, Option<&str>)> =
        NAMES.iter().map(|(name, _value)| (*name, None)).collect();
    assert_eq!(values_by_variable(&built), expected);
}

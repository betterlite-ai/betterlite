use std::collections::BTreeMap;

use secrecy::ExposeSecret as _;

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

#[test]
fn from_lookup_fills_each_field_from_its_variable() {
    let built = WatsonxEnvironment::from_lookup(&environment(&NAMES));

    assert_eq!(built.watsonx_iam_url.as_deref(), Some("watsonx-iam-url"));
    assert_eq!(
        built
            .wx_api_key
            .as_ref()
            .map(|key| key.expose_secret().as_str()),
        Some("wx-api-key")
    );
    assert_eq!(
        built
            .watsonx_api_key
            .as_ref()
            .map(|key| key.expose_secret().as_str()),
        Some("watsonx-api-key")
    );
    assert_eq!(
        built
            .watsonx_apikey
            .as_ref()
            .map(|key| key.expose_secret().as_str()),
        Some("watsonx-apikey")
    );
    assert_eq!(
        built
            .watsonx_zenapikey
            .as_ref()
            .map(|key| key.expose_secret().as_str()),
        Some("watsonx-zenapikey")
    );
    assert_eq!(
        built
            .watsonx_token
            .as_ref()
            .map(|key| key.expose_secret().as_str()),
        Some("watsonx-token")
    );
    assert_eq!(built.watsonx_api_base.as_deref(), Some("watsonx-api-base"));
    assert_eq!(built.watsonx_url.as_deref(), Some("watsonx-url"));
    assert_eq!(built.wx_url.as_deref(), Some("wx-url"));
    assert_eq!(built.wml_url.as_deref(), Some("wml-url"));
    assert_eq!(
        built.watsonx_project_id.as_deref(),
        Some("watsonx-project-id")
    );
    assert_eq!(built.wx_project_id.as_deref(), Some("wx-project-id"));
    assert_eq!(built.project_id.as_deref(), Some("project-id"));
    assert_eq!(built.watsonx_region.as_deref(), Some("watsonx-region"));
    assert_eq!(built.wx_region.as_deref(), Some("wx-region"));
    assert_eq!(built.region.as_deref(), Some("region-name"));
    assert_eq!(
        built.watsonx_deployment_space_id.as_deref(),
        Some("watsonx-deployment-space-id")
    );
    assert_eq!(built.watsonx_space_id.as_deref(), Some("watsonx-space-id"));
    assert_eq!(built.wx_space_id.as_deref(), Some("wx-space-id"));
    assert_eq!(built.space_id.as_deref(), Some("space-id"));
}

#[test]
fn from_lookup_leaves_an_empty_variable_unset() {
    let blanked: Vec<(&str, &str)> = NAMES.iter().map(|(name, _value)| (*name, "")).collect();
    let built = WatsonxEnvironment::from_lookup(&environment(&blanked));

    assert_eq!(built.watsonx_iam_url, None);
    assert!(built.wx_api_key.is_none());
    assert!(built.watsonx_api_key.is_none());
    assert!(built.watsonx_apikey.is_none());
    assert!(built.watsonx_zenapikey.is_none());
    assert!(built.watsonx_token.is_none());
    assert_eq!(built.watsonx_api_base, None);
    assert_eq!(built.watsonx_url, None);
    assert_eq!(built.wx_url, None);
    assert_eq!(built.wml_url, None);
    assert_eq!(built.watsonx_project_id, None);
    assert_eq!(built.wx_project_id, None);
    assert_eq!(built.project_id, None);
    assert_eq!(built.watsonx_region, None);
    assert_eq!(built.wx_region, None);
    assert_eq!(built.region, None);
    assert_eq!(built.watsonx_deployment_space_id, None);
    assert_eq!(built.watsonx_space_id, None);
    assert_eq!(built.wx_space_id, None);
    assert_eq!(built.space_id, None);
}

#[test]
fn from_lookup_leaves_a_missing_variable_unset() {
    let built = WatsonxEnvironment::from_lookup(&environment(&[]));

    assert_eq!(built.watsonx_iam_url, None);
    assert!(built.wx_api_key.is_none());
    assert!(built.watsonx_api_key.is_none());
    assert!(built.watsonx_apikey.is_none());
    assert!(built.watsonx_zenapikey.is_none());
    assert!(built.watsonx_token.is_none());
    assert_eq!(built.watsonx_api_base, None);
    assert_eq!(built.watsonx_url, None);
    assert_eq!(built.wx_url, None);
    assert_eq!(built.wml_url, None);
    assert_eq!(built.watsonx_project_id, None);
    assert_eq!(built.wx_project_id, None);
    assert_eq!(built.project_id, None);
    assert_eq!(built.watsonx_region, None);
    assert_eq!(built.wx_region, None);
    assert_eq!(built.region, None);
    assert_eq!(built.watsonx_deployment_space_id, None);
    assert_eq!(built.watsonx_space_id, None);
    assert_eq!(built.wx_space_id, None);
    assert_eq!(built.space_id, None);
}

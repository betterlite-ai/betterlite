#![cfg(test)]

use std::collections::BTreeMap;

use crate::loader::ConfigLoader;
use secrecy::ExposeSecret as _;

fn load(source: &str) -> crate::sections::bedrock_aws_settings::Settings {
    ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(source)
    .unwrap()
    .sections
    .bedrock_aws_settings
}

#[test]
fn case_1_loads_s3_access_key_id() {
    let settings = load(
        "model_list:\n- model_name: m\n  litellm_params: {model: bedrock/m, s3_access_key_id: configured}\n",
    );
    assert_eq!(
        settings.deployments[0].access_key_id.as_deref(),
        Some("configured")
    );
    assert_eq!(settings.global.access_key_id, None);
}

#[test]
fn case_2_loads_s3_secret_access_key() {
    let settings = load(
        "model_list:\n- model_name: m\n  litellm_params: {model: bedrock/m, s3_secret_access_key: configured}\n",
    );
    assert_eq!(
        settings.deployments[0]
            .secret_access_key
            .as_ref()
            .map(|secret| secret.expose_secret()),
        Some("configured")
    );
    assert!(settings.global.secret_access_key.is_none());
}

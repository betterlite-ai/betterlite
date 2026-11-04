#![cfg(test)]

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use std::collections::BTreeMap;

use crate::loader::ConfigLoader;
use crate::sections::bedrock_aws_settings::AwsS3Settings;

#[rstest]
#[case::loads_s3_access_key_id("s3_access_key_id", |settings: &AwsS3Settings| settings.access_key_id.clone())]
#[case::loads_s3_secret_access_key("s3_secret_access_key", |settings: &AwsS3Settings| settings.secret_access_key.clone())]
fn bedrock_aws_settings_config_cases(
    #[case] key: &str,
    #[case] read: fn(&AwsS3Settings) -> Option<SecretString>,
) {
    let loader = ConfigLoader {
        lookup: BTreeMap::from([
            ("CUSTOM_SECRET".to_owned(), "resolved".to_owned()),
            ("S3_ACCESS_KEY_ID".to_owned(), "ignored".to_owned()),
            ("S3_SECRET_ACCESS_KEY".to_owned(), "ignored".to_owned()),
        ]),
    };
    let source = format!(
        "model_list:\n- model_name: explicit\n  litellm_params: {{model: bedrock/a, {key}: explicit}}\n- model_name: absent\n  litellm_params: {{model: bedrock/b}}\n- model_name: reference\n  litellm_params: {{model: bedrock/c, {key}: os.environ/CUSTOM_SECRET}}\n- model_name: blank\n  litellm_params: {{model: bedrock/d, {key}: ''}}\nlitellm_settings: {{{key}: global}}\n"
    );
    let settings = loader.load(&source).unwrap().sections.bedrock_aws_settings;
    assert_eq!(
        read(&settings.deployments[0]).unwrap().expose_secret(),
        "explicit"
    );
    assert!(read(&settings.deployments[1]).is_none());
    assert_eq!(
        read(&settings.deployments[2]).unwrap().expose_secret(),
        "resolved"
    );
    assert_eq!(read(&settings.deployments[3]).unwrap().expose_secret(), "");
    assert_eq!(read(&settings.global).unwrap().expose_secret(), "global");
    assert!(
        read(
            &loader
                .load("{}")
                .unwrap()
                .sections
                .bedrock_aws_settings
                .global
        )
        .is_none()
    );
    let reference = format!("litellm_settings: {{{key}: os.environ/CUSTOM_SECRET}}");
    assert_eq!(
        read(
            &loader
                .load(&reference)
                .unwrap()
                .sections
                .bedrock_aws_settings
                .global
        )
        .unwrap()
        .expose_secret(),
        "resolved"
    );
    let missing = format!("litellm_settings: {{{key}: os.environ/MISSING}}");
    drop(loader.load(&missing).unwrap_err());
}

#[rstest]
#[case::global("litellm_settings: {s3_unrelated: value}")]
#[case::deployment(
    "model_list: [{model_name: m, litellm_params: {model: bedrock/a, s3_unrelated: value}}]"
)]
fn rejects_unknown(#[case] source: &str) {
    let error = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(source)
    .unwrap_err();
    assert!(error.to_string().contains("s3_unrelated"));
}

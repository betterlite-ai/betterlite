#![cfg(test)]

use std::collections::BTreeMap;

use secrecy::{ExposeSecret as _, SecretString};

use super::{AwsSettings, SessionTagValue};
use crate::loader::{ConfigError, ConfigLoader};

type Read = fn(&AwsSettings) -> Option<String>;

#[derive(Debug, Clone, Copy)]
enum Source {
    Deployment,
    Environment,
    DeploymentOnly,
    Unrelated,
}

fn exposed(secret: &SecretString) -> String {
    secret.expose_secret().to_owned()
}

fn load(params: &str, environment: &[(&str, &str)]) -> Result<Vec<AwsSettings>, ConfigError> {
    let source = format!(
        "model_list:\n- model_name: a\n  litellm_params: {{model: bedrock/a{params}}}\n- model_name: b\n  litellm_params: {{model: bedrock/b}}\n"
    );
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    let config = ConfigLoader { lookup }.load(&source)?;
    Ok(config.sections.bedrock_settings.deployments)
}

fn check(read: Read, params: &str, environment: &[(&str, &str)], expected: [Option<&str>; 2]) {
    let settings = load(params, environment).unwrap();
    let actual = [read(&settings[0]), read(&settings[1])];
    assert_eq!(
        actual.each_ref().map(Option::as_deref),
        expected,
        "{params}"
    );
}

fn entry(pairs: Vec<(&str, SessionTagValue)>) -> SessionTagValue {
    SessionTagValue::Mapping(
        pairs
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

#[rstest::rstest]
#[case::loads_aws_access_key_id(Source::Deployment, "aws_access_key_id", "AWS_ACCESS_KEY_ID", true, |settings: &AwsSettings| settings.access_key_id.as_ref().map(exposed))]
#[case::loads_aws_secret_access_key(Source::Deployment, "aws_secret_access_key", "AWS_SECRET_ACCESS_KEY", true, |settings: &AwsSettings| settings.secret_access_key.as_ref().map(exposed))]
#[case::loads_aws_session_token(Source::Deployment, "aws_session_token", "AWS_SESSION_TOKEN", true, |settings: &AwsSettings| settings.session_token.as_ref().map(exposed))]
#[case::loads_aws_region_name(Source::Deployment, "aws_region_name", "AWS_REGION_NAME", false, |settings: &AwsSettings| settings.region_name.clone())]
#[case::loads_aws_session_name(Source::Deployment, "aws_session_name", "AWS_SESSION_NAME", false, |settings: &AwsSettings| settings.session_name.clone())]
#[case::loads_aws_profile_name(Source::Deployment, "aws_profile_name", "AWS_PROFILE_NAME", false, |settings: &AwsSettings| settings.profile_name.clone())]
#[case::loads_aws_role_name(Source::Deployment, "aws_role_name", "AWS_ROLE_NAME", false, |settings: &AwsSettings| settings.role_name.clone())]
#[case::loads_aws_web_identity_token(Source::Deployment, "aws_web_identity_token", "AWS_WEB_IDENTITY_TOKEN", true, |settings: &AwsSettings| settings.web_identity_token.as_ref().map(exposed))]
#[case::loads_aws_sts_endpoint(Source::Deployment, "aws_sts_endpoint", "AWS_STS_ENDPOINT", false, |settings: &AwsSettings| settings.sts_endpoint.clone())]
#[case::loads_aws_external_id(Source::Deployment, "aws_external_id", "AWS_EXTERNAL_ID", false, |settings: &AwsSettings| settings.external_id.clone())]
#[case::loads_aws_role_arn(Source::Environment, "aws_role_arn", "AWS_ROLE_ARN", false, |settings: &AwsSettings| settings.role_arn.clone())]
#[case::loads_aws_web_identity_token_file(Source::Environment, "aws_web_identity_token_file", "AWS_WEB_IDENTITY_TOKEN_FILE", false, |settings: &AwsSettings| settings.web_identity_token_file.clone())]
#[case::loads_aws_bearer_token_bedrock(Source::Environment, "aws_bearer_token_bedrock", "AWS_BEARER_TOKEN_BEDROCK", true, |settings: &AwsSettings| settings.bearer_token.as_ref().map(exposed))]
#[case::loads_aws_session_tags(Source::DeploymentOnly, "aws_session_tags", "AWS_SESSION_TAGS", false, |settings: &AwsSettings| settings.session_tags.as_ref().map(|tags| format!("{tags:?}")))]
#[case::rejects_unrelated_key(Source::Unrelated, "aws_unrelated_setting", "AWS_UNRELATED_SETTING", false, |_: &AwsSettings| None)]
fn bedrock_settings_config_cases(
    #[case] source: Source,
    #[case] key: &str,
    #[case] variable: &str,
    #[case] secret: bool,
    #[case] read: Read,
) {
    let configured = format!(", {key}: configured");
    let environment = [(variable, "from-env"), ("CUSTOM", "referenced")];
    match source {
        Source::Deployment => {
            check(
                read,
                &configured,
                &environment,
                [Some("configured"), Some("from-env")],
            );
            check(read, &configured, &[], [Some("configured"), None]);
            let empty = format!(", {key}: ''");
            check(read, &empty, &environment, [Some(""), Some("from-env")]);
            let reference = format!(", {key}: os.environ/CUSTOM");
            check(
                read,
                &reference,
                &environment,
                [Some("referenced"), Some("from-env")],
            );
        }
        Source::Environment => {
            check(read, "", &environment, [Some("from-env"), Some("from-env")]);
            check(read, "", &[], [None, None]);
            check(read, "", &[(variable, "")], [Some(""), Some("")]);
        }
        Source::DeploymentOnly => {
            check(
                read,
                &configured,
                &environment,
                [Some("Text(\"configured\")"), None],
            );
            check(read, ", aws_session_tags: null", &[], [None, None]);
            let invalid = ", aws_session_tags: [{Key: 1}, {Value: false}, {Key: a, Key2: ~}, 2.5]";
            assert_eq!(
                load(invalid, &[]).unwrap()[0].session_tags,
                Some(SessionTagValue::List(vec![
                    entry(vec![("Key", SessionTagValue::Integer(1))]),
                    entry(vec![("Value", SessionTagValue::Boolean(false))]),
                    entry(vec![
                        ("Key", SessionTagValue::Text("a".to_owned())),
                        ("Key2", SessionTagValue::Null),
                    ]),
                    SessionTagValue::Decimal(2.5),
                ]))
            );
        }
        Source::Unrelated => {}
    }
    if matches!(source, Source::Environment | Source::Unrelated) {
        let error = load(&configured, &environment).unwrap_err();
        assert!(
            matches!(&error, ConfigError::Unsupported { paths } if *paths == [format!("model_list.0.litellm_params.{key}")]),
            "{error}"
        );
    }
    let debug = format!("{:?}", load("", &environment).unwrap());
    assert_eq!(
        debug.contains("from-env"),
        !secret && matches!(source, Source::Deployment | Source::Environment)
    );
}

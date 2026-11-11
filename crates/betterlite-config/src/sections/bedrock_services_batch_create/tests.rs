#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};

use super::Settings;

#[derive(Clone, Copy)]
enum Expected {
    DeploymentRole(Option<&'static str>),
    DeploymentTags(Option<&'static str>),
    GlobalRole(Option<&'static str>),
    GlobalTags(Option<&'static str>),
    Unrelated(&'static str),
}

fn load(
    deployment: &str,
    global: &str,
    environment: &[(&str, &str)],
) -> Result<Settings, ConfigError> {
    let source = format!(
        "model_list:\n- model_name: oracle\n  litellm_params: {{{deployment}}}\nlitellm_settings: {{{global}}}"
    );
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    ConfigLoader { lookup }
        .load(&source)
        .map(|config| config.sections.bedrock_services_batch_create)
}

fn tags(text: &str) -> Json {
    serde_json::from_str(text).expect("the fixture is valid JSON")
}

#[rstest]
#[case::loads_aws_batch_role_arn(
    "aws_batch_role_arn: arn:aws:iam::111:role/deployment",
    "",
    &[("AWS_BATCH_ROLE_ARN", "arn:aws:iam::222:role/environment")],
    Expected::DeploymentRole(Some("arn:aws:iam::111:role/deployment"))
)]
#[case::loads_aws_batch_role_arn_default("", "", &[], Expected::DeploymentRole(None))]
#[case::loads_aws_batch_role_arn_reference(
    "aws_batch_role_arn: os.environ/BATCH_ROLE",
    "",
    &[("BATCH_ROLE", "arn:aws:iam::333:role/reference")],
    Expected::DeploymentRole(Some("arn:aws:iam::333:role/reference"))
)]
#[case::loads_aws_batch_role_arn_empty_stays_empty(
    "aws_batch_role_arn: ''",
    "",
    &[("AWS_BATCH_ROLE_ARN", "arn:aws:iam::222:role/environment")],
    Expected::DeploymentRole(Some(""))
)]
#[case::loads_aws_batch_role_arn_global(
    "",
    "aws_batch_role_arn: arn:aws:iam::444:role/global",
    &[],
    Expected::GlobalRole(Some("arn:aws:iam::444:role/global"))
)]
#[case::loads_bedrock_tags(
    "bedrock_tags: [{key: team, value: genai}, {key: env, value: prod}]",
    "",
    &[],
    Expected::DeploymentTags(Some(
        r#"[{"key": "team", "value": "genai"}, {"key": "env", "value": "prod"}]"#
    ))
)]
#[case::loads_bedrock_tags_default("", "", &[], Expected::DeploymentTags(None))]
#[case::loads_bedrock_tags_empty("bedrock_tags: []", "", &[], Expected::DeploymentTags(Some("[]")))]
#[case::loads_bedrock_tags_global(
    "",
    "bedrock_tags: [{key: env, value: prod}]",
    &[],
    Expected::GlobalTags(Some(r#"[{"key": "env", "value": "prod"}]"#))
)]
#[case::rejects_unrelated_key(
    "bedrock_unrelated_setting: 1",
    "",
    &[],
    Expected::Unrelated("model_list.0.litellm_params.bedrock_unrelated_setting")
)]
fn bedrock_services_batch_create_config_cases(
    #[case] deployment: &str,
    #[case] global: &str,
    #[case] environment: &[(&str, &str)],
    #[case] expected: Expected,
) {
    match expected {
        Expected::DeploymentRole(role) => {
            let settings = load(deployment, global, environment).expect("the config loads");
            assert_eq!(settings.deployments[0].aws_batch_role_arn.as_deref(), role);
            assert_eq!(settings.global.aws_batch_role_arn, None);
        }
        Expected::DeploymentTags(value) => {
            let settings = load(deployment, global, environment).expect("the config loads");
            assert_eq!(
                settings.deployments[0].bedrock_tags.as_ref(),
                value.map(tags).as_ref()
            );
            assert_eq!(settings.global.bedrock_tags, None);
        }
        Expected::GlobalRole(role) => {
            let settings = load(deployment, global, environment).expect("the config loads");
            assert_eq!(settings.global.aws_batch_role_arn.as_deref(), role);
            assert_eq!(settings.deployments[0].aws_batch_role_arn, None);
        }
        Expected::GlobalTags(value) => {
            let settings = load(deployment, global, environment).expect("the config loads");
            assert_eq!(
                settings.global.bedrock_tags.as_ref(),
                value.map(tags).as_ref()
            );
            assert_eq!(settings.deployments[0].bedrock_tags, None);
        }
        Expected::Unrelated(path) => {
            let error = load(deployment, global, environment).expect_err("the key is unknown");
            let ConfigError::Unsupported { paths } = error else {
                panic!("expected unsupported keys, got {error:?}");
            };
            assert_eq!(paths, [path.to_owned()]);
        }
    }
}

#[rstest]
#[case::loads_aws_batch_role_arn(&[("AWS_BATCH_ROLE_ARN", "arn:aws:iam::222:role/environment")], Some("arn:aws:iam::222:role/environment"))]
#[case::loads_aws_batch_role_arn_unset(&[], None)]
#[case::loads_aws_batch_role_arn_empty(&[("AWS_BATCH_ROLE_ARN", "")], None)]
fn batch_create_environment_config_cases(
    #[case] environment: &[(&str, &str)],
    #[case] expected: Option<&str>,
) {
    let settings = load("", "", environment).expect("the config loads");
    assert_eq!(settings.environment.aws_batch_role_arn.as_deref(), expected);
    assert_eq!(settings.deployments[0].aws_batch_role_arn, None);
    assert_eq!(settings.global.aws_batch_role_arn, None);
}

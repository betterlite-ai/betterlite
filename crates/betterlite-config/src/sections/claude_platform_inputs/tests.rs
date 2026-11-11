#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};

use super::{PlatformInputs, PlatformValue, Settings, UnsupportedParams};
use crate::loader::{ConfigError, ConfigLoader};

fn load(source: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .claude_platform_inputs)
}

fn deployment(key: &str, value: &str) -> String {
    format!(
        "model_list:\n- model_name: unit\n  litellm_params: {{model: bedrock/unit, {key}: {value}}}\n"
    )
}

fn deployment_and_settings(key: &str) -> String {
    format!(
        "model_list:\n- model_name: unit\n  litellm_params: {{model: bedrock/unit, {key}: deploy-value}}\nlitellm_settings: {{{key}: global-value}}\n"
    )
}

fn revealed(value: Option<&SecretString>) -> Option<&str> {
    value.map(ExposeSecret::expose_secret)
}

type Read = fn(&PlatformInputs) -> Option<String>;

fn assert_workspace_alias(key: &str, read: Read) {
    let defaults = load("", &[]).unwrap();
    assert_eq!(read(&defaults.litellm_settings), None, "{key} default");
    assert!(defaults.deployments.is_empty());
    let deployment_only = load(&deployment(key, "deploy-value"), &[]).unwrap();
    assert_eq!(
        read(deployment_only.deployments.first().unwrap()),
        Some("deploy-value".to_owned()),
        "{key} deployment"
    );
    let global_only = load(&format!("litellm_settings: {{{key}: global-value}}\n"), &[]).unwrap();
    assert_eq!(
        read(&global_only.litellm_settings),
        Some("global-value".to_owned()),
        "{key} global"
    );
    let both = load(&deployment_and_settings(key), &[]).unwrap();
    assert_eq!(
        read(both.deployments.first().unwrap()),
        Some("deploy-value".to_owned()),
        "{key} keeps the deployment value"
    );
    assert_eq!(
        read(&both.litellm_settings),
        Some("global-value".to_owned()),
        "{key} keeps the module value"
    );
    let empty = load(&deployment(key, "''"), &[]).unwrap();
    assert_eq!(
        read(empty.deployments.first().unwrap()),
        Some(String::new()),
        "{key} keeps an empty value"
    );
    let unrelated_key = format!("{key}_unrelated");
    let error = load(&deployment(&unrelated_key, "1"), &[]).unwrap_err();
    assert!(
        matches!(&error, ConfigError::Unsupported { paths } if *paths == [format!("model_list.0.litellm_params.{unrelated_key}")]),
        "{error}"
    );
}

#[rstest]
#[case::loads_workspace_id("workspace_id", |inputs: &PlatformInputs| inputs.workspace_id.clone())]
#[case::loads_aws_workspace_id("aws_workspace_id", |inputs: &PlatformInputs| inputs.aws_workspace_id.clone())]
#[case::loads_anthropic_hyphen_workspace_hyphen_id("anthropic-workspace-id", |inputs: &PlatformInputs| inputs.anthropic_hyphen_workspace_id.clone())]
#[case::loads_anthropic_workspace_id("anthropic_workspace_id", |inputs: &PlatformInputs| inputs.anthropic_workspace_id.clone())]
fn claude_workspace_aliases_config_cases(#[case] key: &str, #[case] read: Read) {
    assert_workspace_alias(key, read);
}

#[derive(Clone, Copy)]
enum Case {
    AnthropicAwsWorkspaceId,
    AnthropicWorkspaceId,
    AnthropicAwsBaseUrl,
    AnthropicAwsApiBase,
    AnthropicAwsApiKey,
    UnsupportedParams,
    RejectsUnrelatedKey,
}

#[rstest]
#[case::loads_anthropic_aws_workspace_id(Case::AnthropicAwsWorkspaceId)]
#[case::loads_anthropic_workspace_id(Case::AnthropicWorkspaceId)]
#[case::loads_anthropic_aws_base_url(Case::AnthropicAwsBaseUrl)]
#[case::loads_anthropic_aws_api_base(Case::AnthropicAwsApiBase)]
#[case::loads_anthropic_aws_api_key(Case::AnthropicAwsApiKey)]
#[case::loads_claude_platform_unsupported_params(Case::UnsupportedParams)]
#[case::rejects_unrelated_key(Case::RejectsUnrelatedKey)]
fn claude_platform_inputs_config_cases(#[case] case: Case) {
    match case {
        Case::AnthropicAwsWorkspaceId => {
            let default = load("", &[]).unwrap();
            assert_eq!(
                revealed(default.environment.anthropic_aws_workspace_id.as_ref()),
                None
            );
            let set = load("", &[("ANTHROPIC_AWS_WORKSPACE_ID", "fixture-workspace")]).unwrap();
            assert_eq!(
                revealed(set.environment.anthropic_aws_workspace_id.as_ref()),
                Some("fixture-workspace")
            );
            assert!(!format!("{:?}", set.environment).contains("fixture-workspace"));
            let empty = load("", &[("ANTHROPIC_AWS_WORKSPACE_ID", "")]).unwrap();
            assert_eq!(
                revealed(empty.environment.anthropic_aws_workspace_id.as_ref()),
                Some("")
            );
            let boolean = load("", &[("ANTHROPIC_AWS_WORKSPACE_ID", "true")]).unwrap();
            assert_eq!(
                revealed(boolean.environment.anthropic_aws_workspace_id.as_ref()),
                None
            );
        }
        Case::AnthropicWorkspaceId => {
            let default = load("", &[]).unwrap();
            assert_eq!(
                revealed(default.environment.anthropic_workspace_id.as_ref()),
                None
            );
            let set = load("", &[("ANTHROPIC_WORKSPACE_ID", "fixture-workspace")]).unwrap();
            assert_eq!(
                revealed(set.environment.anthropic_workspace_id.as_ref()),
                Some("fixture-workspace")
            );
            assert!(!format!("{:?}", set.environment).contains("fixture-workspace"));
            let empty = load("", &[("ANTHROPIC_WORKSPACE_ID", "")]).unwrap();
            assert_eq!(
                revealed(empty.environment.anthropic_workspace_id.as_ref()),
                Some("")
            );
            let boolean = load("", &[("ANTHROPIC_WORKSPACE_ID", "false")]).unwrap();
            assert_eq!(
                revealed(boolean.environment.anthropic_workspace_id.as_ref()),
                None
            );
        }
        Case::AnthropicAwsBaseUrl => {
            let default = load("", &[]).unwrap();
            assert_eq!(default.environment.anthropic_aws_base_url, None);
            let set = load("", &[("ANTHROPIC_AWS_BASE_URL", "https://base.example")]).unwrap();
            assert_eq!(
                set.environment.anthropic_aws_base_url.as_deref(),
                Some("https://base.example")
            );
            let empty = load("", &[("ANTHROPIC_AWS_BASE_URL", "")]).unwrap();
            assert_eq!(
                empty.environment.anthropic_aws_base_url.as_deref(),
                Some("")
            );
        }
        Case::AnthropicAwsApiBase => {
            let default = load("", &[]).unwrap();
            assert_eq!(default.environment.anthropic_aws_api_base, None);
            let set = load("", &[("ANTHROPIC_AWS_API_BASE", "https://api.example")]).unwrap();
            assert_eq!(
                set.environment.anthropic_aws_api_base.as_deref(),
                Some("https://api.example")
            );
            let empty = load("", &[("ANTHROPIC_AWS_API_BASE", "")]).unwrap();
            assert_eq!(
                empty.environment.anthropic_aws_api_base.as_deref(),
                Some("")
            );
        }
        Case::AnthropicAwsApiKey => {
            let default = load("", &[]).unwrap();
            assert_eq!(
                revealed(default.environment.anthropic_aws_api_key.as_ref()),
                None
            );
            let set = load("", &[("ANTHROPIC_AWS_API_KEY", "fixture-key")]).unwrap();
            assert_eq!(
                revealed(set.environment.anthropic_aws_api_key.as_ref()),
                Some("fixture-key")
            );
            assert!(!format!("{:?}", set.environment).contains("fixture-key"));
            let empty = load("", &[("ANTHROPIC_AWS_API_KEY", "")]).unwrap();
            assert_eq!(
                revealed(empty.environment.anthropic_aws_api_key.as_ref()),
                Some("")
            );
        }
        Case::UnsupportedParams => {
            let key = "claude_platform_unsupported_params";
            let default = load("", &[]).unwrap();
            assert_eq!(default.litellm_settings.unsupported_params, None);
            let deployment_only =
                load(&deployment(key, "[context_management, cache_control]"), &[]).unwrap();
            assert_eq!(
                deployment_only
                    .deployments
                    .first()
                    .unwrap()
                    .unsupported_params,
                Some(UnsupportedParams::List(vec![
                    PlatformValue::Text("context_management".to_owned()),
                    PlatformValue::Text("cache_control".to_owned()),
                ]))
            );
            let global_only = load(
                "litellm_settings: {claude_platform_unsupported_params: [context_management]}\n",
                &[],
            )
            .unwrap();
            assert_eq!(
                global_only.litellm_settings.unsupported_params,
                Some(UnsupportedParams::List(vec![PlatformValue::Text(
                    "context_management".to_owned()
                )]))
            );
            let both = load(&deployment_and_settings(key), &[]).unwrap();
            assert_eq!(
                both.deployments.first().unwrap().unsupported_params,
                Some(UnsupportedParams::Other(PlatformValue::Text(
                    "deploy-value".to_owned()
                )))
            );
            assert_eq!(
                both.litellm_settings.unsupported_params,
                Some(UnsupportedParams::Other(PlatformValue::Text(
                    "global-value".to_owned()
                )))
            );
            let scalar = load(&deployment(key, "not-a-list"), &[]).unwrap();
            assert_eq!(
                scalar.deployments.first().unwrap().unsupported_params,
                Some(UnsupportedParams::Other(PlatformValue::Text(
                    "not-a-list".to_owned()
                )))
            );
            let empty = load(&deployment(key, "[]"), &[]).unwrap();
            assert_eq!(
                empty.deployments.first().unwrap().unsupported_params,
                Some(UnsupportedParams::List(Vec::new()))
            );
        }
        Case::RejectsUnrelatedKey => {
            let error = load(&deployment("workspace_id_unrelated", "1"), &[]).unwrap_err();
            assert!(
                matches!(&error, ConfigError::Unsupported { paths } if *paths == ["model_list.0.litellm_params.workspace_id_unrelated"]),
                "{error}"
            );
            let global_error =
                load("litellm_settings: {claude_platform_unrelated: true}\n", &[]).unwrap_err();
            assert!(
                matches!(&global_error, ConfigError::Unsupported { paths } if *paths == ["litellm_settings.claude_platform_unrelated"]),
                "{global_error}"
            );
        }
    }
}

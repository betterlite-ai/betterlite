#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::ExposeSecret as _;
use secrecy::SecretString;

use crate::loader::ConfigLoader;

use super::Settings;

const EMPTY: &str = "litellm_settings: {}\n";
const NULL: &str = "litellm_settings: null\n";

#[derive(Clone, Copy)]
enum Field {
    AnthropicApiBase,
    AnthropicApiKey,
    AnthropicBase,
    ApiBase,
    ApiKey,
    ApiVersion,
    AzureAdToken,
    AzureApiBase,
    AzureApiKey,
    AzureApiVersion,
    AzureKey,
    AzureOpenaiApiKey,
    OpenaiApiBase,
    OpenaiApiKey,
    OpenaiBaseUrl,
    OpenaiKey,
    OpenaiOrganization,
    Organization,
    VertexaiCredentials,
    VertexaiLocation,
    VertexaiProject,
    VertexLocation,
    VertexProject,
}

fn exposed(secret: Option<&SecretString>) -> Option<String> {
    let text = secret?;
    Some(text.expose_secret().to_owned())
}

fn value(settings: &Settings, field: Field) -> Option<String> {
    let resources = &settings.resources;
    match field {
        Field::AnthropicApiBase => settings.anthropic_api_base.clone(),
        Field::AnthropicApiKey => exposed(settings.anthropic_api_key.as_ref()),
        Field::AnthropicBase => settings.anthropic_base().map(str::to_owned),
        Field::ApiBase => resources.api_base.clone(),
        Field::ApiKey => exposed(resources.api_key.as_ref()),
        Field::ApiVersion => resources.api_version.clone(),
        Field::AzureAdToken => exposed(resources.azure_ad_token.as_ref()),
        Field::AzureApiBase => resources.azure_api_base.clone(),
        Field::AzureApiKey => exposed(resources.azure_api_key.as_ref()),
        Field::AzureApiVersion => resources.azure_api_version.clone(),
        Field::AzureKey => exposed(resources.azure_key.as_ref()),
        Field::AzureOpenaiApiKey => exposed(resources.azure_openai_api_key.as_ref()),
        Field::OpenaiApiBase => resources.openai_api_base.clone(),
        Field::OpenaiApiKey => exposed(resources.openai_api_key.as_ref()),
        Field::OpenaiBaseUrl => resources.openai_base_url.clone(),
        Field::OpenaiKey => exposed(resources.openai_key.as_ref()),
        Field::OpenaiOrganization => resources.openai_organization.clone(),
        Field::Organization => resources.organization.clone(),
        Field::VertexaiCredentials => exposed(resources.vertexai_credentials.as_ref()),
        Field::VertexaiLocation => resources.vertexai_location.clone(),
        Field::VertexaiProject => resources.vertexai_project.clone(),
        Field::VertexLocation => resources.vertex_location.clone(),
        Field::VertexProject => resources.vertex_project.clone(),
    }
}

type Case = Result<(Field, Option<&'static str>), &'static str>;

#[rstest]
#[case::api_key_from_yaml("litellm_settings:\n  api_key: yaml-key\n", &[], Ok((Field::ApiKey, Some("yaml-key"))))]
#[case::api_key_from_environment_reference("litellm_settings:\n  api_key: os.environ/ORACLE_KEY\n", &[("ORACLE_KEY", "env-key")], Ok((Field::ApiKey, Some("env-key"))))]
#[case::api_key_missing_environment("litellm_settings:\n  api_key: os.environ/ABSENT_KEY\n", &[], Err("missing_environment"))]
#[case::api_key_invalid_type("litellm_settings:\n  api_key:\n    nested: key\n", &[], Err("yaml"))]
#[case::api_key_unset(EMPTY, &[], Ok((Field::ApiKey, None)))]
#[case::openai_key_from_yaml("litellm_settings:\n  openai_key: yaml-openai\n", &[], Ok((Field::OpenaiKey, Some("yaml-openai"))))]
#[case::openai_key_invalid_type("litellm_settings:\n  openai_key: [key]\n", &[], Err("yaml"))]
#[case::openai_key_unset(EMPTY, &[], Ok((Field::OpenaiKey, None)))]
#[case::openai_api_key_environment(EMPTY, &[("OPENAI_API_KEY", "env-key")], Ok((Field::OpenaiApiKey, Some("env-key"))))]
#[case::openai_api_key_empty_environment(EMPTY, &[("OPENAI_API_KEY", "")], Ok((Field::OpenaiApiKey, Some(""))))]
#[case::openai_api_key_unset(EMPTY, &[], Ok((Field::OpenaiApiKey, None)))]
#[case::api_base_from_yaml("litellm_settings:\n  api_base: https://yaml.example\n", &[], Ok((Field::ApiBase, Some("https://yaml.example"))))]
#[case::api_base_from_environment_reference("litellm_settings:\n  api_base: os.environ/ORACLE_BASE\n", &[("ORACLE_BASE", "https://reference.example")], Ok((Field::ApiBase, Some("https://reference.example"))))]
#[case::api_base_invalid_type("litellm_settings:\n  api_base: [https://yaml.example]\n", &[], Err("yaml"))]
#[case::api_base_unset(EMPTY, &[], Ok((Field::ApiBase, None)))]
#[case::openai_base_url_environment(EMPTY, &[("OPENAI_BASE_URL", "https://base-url.example")], Ok((Field::OpenaiBaseUrl, Some("https://base-url.example"))))]
#[case::openai_base_url_unset(EMPTY, &[], Ok((Field::OpenaiBaseUrl, None)))]
#[case::openai_api_base_environment(EMPTY, &[("OPENAI_API_BASE", "https://api-base.example")], Ok((Field::OpenaiApiBase, Some("https://api-base.example"))))]
#[case::openai_api_base_unset(EMPTY, &[], Ok((Field::OpenaiApiBase, None)))]
#[case::organization_from_yaml("litellm_settings:\n  organization: yaml-org\n", &[], Ok((Field::Organization, Some("yaml-org"))))]
#[case::organization_invalid_type("litellm_settings:\n  organization:\n    - org\n", &[], Err("yaml"))]
#[case::organization_unset(EMPTY, &[], Ok((Field::Organization, None)))]
#[case::openai_organization_environment(EMPTY, &[("OPENAI_ORGANIZATION", "env-org")], Ok((Field::OpenaiOrganization, Some("env-org"))))]
#[case::openai_organization_unset(EMPTY, &[], Ok((Field::OpenaiOrganization, None)))]
#[case::api_version_from_yaml("litellm_settings:\n  api_version: 2024-01-01\n", &[], Ok((Field::ApiVersion, Some("2024-01-01"))))]
#[case::api_version_invalid_type("litellm_settings:\n  api_version:\n    - 2024-01-01\n", &[], Err("yaml"))]
#[case::api_version_unset(EMPTY, &[], Ok((Field::ApiVersion, None)))]
#[case::azure_api_version_environment(EMPTY, &[("AZURE_API_VERSION", "2024-06-01")], Ok((Field::AzureApiVersion, Some("2024-06-01"))))]
#[case::azure_api_version_unset(EMPTY, &[], Ok((Field::AzureApiVersion, None)))]
#[case::azure_key_from_yaml("litellm_settings:\n  azure_key: yaml-azure\n", &[], Ok((Field::AzureKey, Some("yaml-azure"))))]
#[case::azure_key_invalid_type("litellm_settings:\n  azure_key:\n    nested: key\n", &[], Err("yaml"))]
#[case::azure_key_unset(EMPTY, &[], Ok((Field::AzureKey, None)))]
#[case::azure_api_base_environment(EMPTY, &[("AZURE_API_BASE", "https://azure.example")], Ok((Field::AzureApiBase, Some("https://azure.example"))))]
#[case::azure_api_base_unset(EMPTY, &[], Ok((Field::AzureApiBase, None)))]
#[case::azure_openai_api_key_environment(EMPTY, &[("AZURE_OPENAI_API_KEY", "env-azure-openai")], Ok((Field::AzureOpenaiApiKey, Some("env-azure-openai"))))]
#[case::azure_openai_api_key_unset(EMPTY, &[], Ok((Field::AzureOpenaiApiKey, None)))]
#[case::azure_api_key_environment(EMPTY, &[("AZURE_API_KEY", "env-azure")], Ok((Field::AzureApiKey, Some("env-azure"))))]
#[case::azure_api_key_empty_environment(EMPTY, &[("AZURE_API_KEY", "")], Ok((Field::AzureApiKey, Some(""))))]
#[case::azure_api_key_unset(EMPTY, &[], Ok((Field::AzureApiKey, None)))]
#[case::azure_ad_token_environment(EMPTY, &[("AZURE_AD_TOKEN", "env-token")], Ok((Field::AzureAdToken, Some("env-token"))))]
#[case::azure_ad_token_unset(EMPTY, &[], Ok((Field::AzureAdToken, None)))]
#[case::vertex_project_from_yaml("litellm_settings:\n  vertex_project: yaml-project\n", &[], Ok((Field::VertexProject, Some("yaml-project"))))]
#[case::vertex_project_invalid_type("litellm_settings:\n  vertex_project:\n    - project\n", &[], Err("yaml"))]
#[case::vertex_project_unset(EMPTY, &[], Ok((Field::VertexProject, None)))]
#[case::vertexai_project_environment(EMPTY, &[("VERTEXAI_PROJECT", "env-project")], Ok((Field::VertexaiProject, Some("env-project"))))]
#[case::vertexai_project_unset(EMPTY, &[], Ok((Field::VertexaiProject, None)))]
#[case::vertex_location_from_yaml("litellm_settings:\n  vertex_location: yaml-location\n", &[], Ok((Field::VertexLocation, Some("yaml-location"))))]
#[case::vertex_location_invalid_type("litellm_settings:\n  vertex_location:\n    - location\n", &[], Err("yaml"))]
#[case::vertex_location_unset(EMPTY, &[], Ok((Field::VertexLocation, None)))]
#[case::vertexai_location_environment(EMPTY, &[("VERTEXAI_LOCATION", "env-location")], Ok((Field::VertexaiLocation, Some("env-location"))))]
#[case::vertexai_location_unset(EMPTY, &[], Ok((Field::VertexaiLocation, None)))]
#[case::vertexai_credentials_environment(EMPTY, &[("VERTEXAI_CREDENTIALS", "env-credentials")], Ok((Field::VertexaiCredentials, Some("env-credentials"))))]
#[case::vertexai_credentials_unset(EMPTY, &[], Ok((Field::VertexaiCredentials, None)))]
#[case::anthropic_api_base_environment(EMPTY, &[("ANTHROPIC_API_BASE", "https://api-base.example")], Ok((Field::AnthropicApiBase, Some("https://api-base.example"))))]
#[case::anthropic_api_base_unset(EMPTY, &[], Ok((Field::AnthropicApiBase, None)))]
#[case::anthropic_base_url_environment(EMPTY, &[("ANTHROPIC_BASE_URL", "https://base-url.example")], Ok((Field::AnthropicBase, Some("https://base-url.example"))))]
#[case::anthropic_base_prefers_api_base(EMPTY, &[("ANTHROPIC_API_BASE", "https://api-base.example"), ("ANTHROPIC_BASE_URL", "https://base-url.example")], Ok((Field::AnthropicBase, Some("https://api-base.example"))))]
#[case::anthropic_base_skips_empty_api_base(EMPTY, &[("ANTHROPIC_API_BASE", ""), ("ANTHROPIC_BASE_URL", "https://base-url.example")], Ok((Field::AnthropicBase, Some("https://base-url.example"))))]
#[case::anthropic_base_skips_empty_base_url(EMPTY, &[("ANTHROPIC_BASE_URL", "")], Ok((Field::AnthropicBase, None)))]
#[case::anthropic_base_unset(EMPTY, &[], Ok((Field::AnthropicBase, None)))]
#[case::anthropic_api_key_environment(EMPTY, &[("ANTHROPIC_API_KEY", "env-anthropic")], Ok((Field::AnthropicApiKey, Some("env-anthropic"))))]
#[case::anthropic_api_key_empty_environment(EMPTY, &[("ANTHROPIC_API_KEY", "")], Ok((Field::AnthropicApiKey, Some(""))))]
#[case::anthropic_api_key_unset(EMPTY, &[], Ok((Field::AnthropicApiKey, None)))]
#[case::null_settings(NULL, &[], Ok((Field::ApiKey, None)))]
#[case::null_settings_still_reads_environment(NULL, &[("ANTHROPIC_API_KEY", "env-anthropic")], Ok((Field::AnthropicApiKey, Some("env-anthropic"))))]
fn loads_settings(
    #[case] yaml: &str,
    #[case] environment: &[(&str, &str)],
    #[case] expected: Case,
) {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    match expected {
        Ok((field, wanted)) => {
            let config = ConfigLoader { lookup }.load(yaml).expect("settings load");
            assert_eq!(
                value(&config.sections.batch_provider_read, field).as_deref(),
                wanted
            );
        }
        Err(category) => assert_eq!(
            ConfigLoader { lookup }.load(yaml).unwrap_err().category(),
            category
        ),
    }
}

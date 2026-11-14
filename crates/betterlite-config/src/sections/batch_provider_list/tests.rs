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

fn exposed(value: &Option<SecretString>) -> Option<String> {
    value.as_ref().map(|text| text.expose_secret().clone())
}

fn value(settings: &Settings, field: Field) -> Option<String> {
    match field {
        Field::ApiBase => settings.api_base.clone(),
        Field::ApiKey => exposed(&settings.api_key),
        Field::ApiVersion => settings.api_version.clone(),
        Field::AzureAdToken => exposed(&settings.azure_ad_token),
        Field::AzureApiBase => settings.azure_api_base.clone(),
        Field::AzureApiKey => exposed(&settings.azure_api_key),
        Field::AzureApiVersion => settings.azure_api_version.clone(),
        Field::AzureKey => exposed(&settings.azure_key),
        Field::AzureOpenaiApiKey => exposed(&settings.azure_openai_api_key),
        Field::OpenaiApiBase => settings.openai_api_base.clone(),
        Field::OpenaiApiKey => exposed(&settings.openai_api_key),
        Field::OpenaiBaseUrl => settings.openai_base_url.clone(),
        Field::OpenaiKey => exposed(&settings.openai_key),
        Field::OpenaiOrganization => settings.openai_organization.clone(),
        Field::Organization => settings.organization.clone(),
        Field::VertexaiCredentials => exposed(&settings.vertexai_credentials),
        Field::VertexaiLocation => settings.vertexai_location.clone(),
        Field::VertexaiProject => settings.vertexai_project.clone(),
        Field::VertexLocation => settings.vertex_location.clone(),
        Field::VertexProject => settings.vertex_project.clone(),
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
#[case::openai_key_from_environment_reference("litellm_settings:\n  openai_key: os.environ/ORACLE_OPENAI_KEY\n", &[("ORACLE_OPENAI_KEY", "env-openai")], Ok((Field::OpenaiKey, Some("env-openai"))))]
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
#[case::openai_api_base_environment(EMPTY, &[("OPENAI_API_BASE", "https://api-base.example")], Ok((Field::OpenaiApiBase, Some("https://api-base.example"))))]
#[case::openai_bases_unset(EMPTY, &[], Ok((Field::OpenaiBaseUrl, None)))]
#[case::organization_from_yaml("litellm_settings:\n  organization: yaml-org\n", &[], Ok((Field::Organization, Some("yaml-org"))))]
#[case::organization_unset(EMPTY, &[], Ok((Field::Organization, None)))]
#[case::openai_organization_environment(EMPTY, &[("OPENAI_ORGANIZATION", "env-org")], Ok((Field::OpenaiOrganization, Some("env-org"))))]
#[case::openai_organization_unset(EMPTY, &[], Ok((Field::OpenaiOrganization, None)))]
#[case::api_version_from_yaml("litellm_settings:\n  api_version: 2024-01-01\n", &[], Ok((Field::ApiVersion, Some("2024-01-01"))))]
#[case::api_version_unset(EMPTY, &[], Ok((Field::ApiVersion, None)))]
#[case::azure_api_version_environment(EMPTY, &[("AZURE_API_VERSION", "2024-06-01")], Ok((Field::AzureApiVersion, Some("2024-06-01"))))]
#[case::azure_api_version_unset(EMPTY, &[], Ok((Field::AzureApiVersion, None)))]
#[case::azure_key_from_yaml("litellm_settings:\n  azure_key: yaml-azure\n", &[], Ok((Field::AzureKey, Some("yaml-azure"))))]
#[case::azure_key_unset(EMPTY, &[], Ok((Field::AzureKey, None)))]
#[case::azure_api_base_environment(EMPTY, &[("AZURE_API_BASE", "https://azure.example")], Ok((Field::AzureApiBase, Some("https://azure.example"))))]
#[case::azure_api_base_unset(EMPTY, &[], Ok((Field::AzureApiBase, None)))]
#[case::azure_openai_api_key_environment(EMPTY, &[("AZURE_OPENAI_API_KEY", "env-azure-openai")], Ok((Field::AzureOpenaiApiKey, Some("env-azure-openai"))))]
#[case::azure_openai_api_key_unset(EMPTY, &[], Ok((Field::AzureOpenaiApiKey, None)))]
#[case::azure_api_key_environment(EMPTY, &[("AZURE_API_KEY", "env-azure")], Ok((Field::AzureApiKey, Some("env-azure"))))]
#[case::azure_api_key_unset(EMPTY, &[], Ok((Field::AzureApiKey, None)))]
#[case::azure_ad_token_environment(EMPTY, &[("AZURE_AD_TOKEN", "env-token")], Ok((Field::AzureAdToken, Some("env-token"))))]
#[case::azure_ad_token_unset(EMPTY, &[], Ok((Field::AzureAdToken, None)))]
#[case::vertex_project_from_yaml("litellm_settings:\n  vertex_project: yaml-project\n", &[], Ok((Field::VertexProject, Some("yaml-project"))))]
#[case::vertex_project_invalid_type("litellm_settings:\n  vertex_project:\n    - project\n", &[], Err("yaml"))]
#[case::vertex_project_unset(EMPTY, &[], Ok((Field::VertexProject, None)))]
#[case::vertexai_project_environment(EMPTY, &[("VERTEXAI_PROJECT", "env-project")], Ok((Field::VertexaiProject, Some("env-project"))))]
#[case::vertexai_project_unset(EMPTY, &[], Ok((Field::VertexaiProject, None)))]
#[case::vertex_location_from_yaml("litellm_settings:\n  vertex_location: yaml-location\n", &[], Ok((Field::VertexLocation, Some("yaml-location"))))]
#[case::vertex_location_unset(EMPTY, &[], Ok((Field::VertexLocation, None)))]
#[case::vertexai_location_environment(EMPTY, &[("VERTEXAI_LOCATION", "env-location")], Ok((Field::VertexaiLocation, Some("env-location"))))]
#[case::vertexai_location_unset(EMPTY, &[], Ok((Field::VertexaiLocation, None)))]
#[case::vertexai_credentials_environment(EMPTY, &[("VERTEXAI_CREDENTIALS", "env-credentials")], Ok((Field::VertexaiCredentials, Some("env-credentials"))))]
#[case::vertexai_credentials_unset(EMPTY, &[], Ok((Field::VertexaiCredentials, None)))]
#[case::null_settings(NULL, &[], Ok((Field::ApiKey, None)))]
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
        Ok((field, expected)) => {
            let config = ConfigLoader { lookup }.load(yaml).expect("settings load");
            assert_eq!(
                value(&config.sections.batch_provider_list, field).as_deref(),
                expected
            );
        }
        Err(category) => assert_eq!(
            ConfigLoader { lookup }.load(yaml).unwrap_err().category(),
            category
        ),
    }
}

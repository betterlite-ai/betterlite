#![cfg(test)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::loader::{ConfigError, ConfigLoader, EnvironmentLookup};
use crate::sections::vertex_credential_sources::VertexSettings;
use crate::sections::vertex_credential_sources::credential_source::aws_params::ExternalCredentialSource;
use crate::sections::vertex_credential_sources::credential_source::{
    ConfiguredCredentials, CredentialSource,
};

struct Lookup {
    variables: BTreeMap<String, String>,
    home: Option<PathBuf>,
}

impl EnvironmentLookup for Lookup {
    fn lookup(&self, name: &str) -> Option<String> {
        self.variables.get(name).cloned()
    }

    fn system_home(&self) -> Option<PathBuf> {
        self.home.clone()
    }
}

fn load(
    source: &str,
    environment: &[(&str, &str)],
    home: Option<&str>,
) -> Result<VertexSettings, ConfigError> {
    let lookup = Lookup {
        variables: environment
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect(),
        home: home.map(PathBuf::from),
    };
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .vertex_credential_sources)
}

#[test]
fn loads_vertex_sources() {
    let absent = load("", &[], None).unwrap();
    assert!(absent.project.is_none());
    assert!(absent.location.is_none());
    assert!(absent.credentials.is_none());

    let module = load(
        "litellm_settings:\n  vertex_project: module-project\n  vertex_location: module-location\n",
        &[("VERTEXAI_PROJECT", "env-project")],
        None,
    )
    .unwrap();
    assert_eq!(module.project.as_deref(), Some("module-project"));
    assert_eq!(module.location.as_deref(), Some("module-location"));

    let environment = load(
        "",
        &[
            ("VERTEXAI_PROJECT", "env-project"),
            ("VERTEXAI_LOCATION", "env-location"),
            ("VERTEX_LOCATION", "legacy-location"),
        ],
        None,
    )
    .unwrap();
    assert_eq!(environment.project.as_deref(), Some("env-project"));
    assert_eq!(environment.location.as_deref(), Some("env-location"));
}

#[test]
fn loads_the_legacy_vertex_location_fallback() {
    let legacy = load("", &[("VERTEX_LOCATION", "legacy-location")], None).unwrap();
    assert_eq!(legacy.location.as_deref(), Some("legacy-location"));
}

#[test]
fn reads_an_empty_vertex_environment_as_unset() {
    let empty = load(
        "",
        &[("VERTEXAI_PROJECT", ""), ("VERTEXAI_LOCATION", "")],
        None,
    )
    .unwrap();
    assert!(empty.project.is_none());
    assert!(empty.location.is_none());
}

#[test]
fn keeps_an_explicit_empty_vertex_project() {
    let explicit = load("litellm_settings:\n  vertex_project: \"\"\n", &[], None).unwrap();
    assert_eq!(explicit.project.as_deref(), Some(""));
}

#[test]
fn resolves_vertex_os_environ_references() {
    let resolved = load(
        "litellm_settings:\n  vertex_project: os.environ/PROJECT_REFERENCE\n  vertex_location: os.environ/LOCATION_REFERENCE\n",
        &[
            ("PROJECT_REFERENCE", "referenced-project"),
            ("LOCATION_REFERENCE", "referenced-location"),
        ],
        None,
    )
    .unwrap();
    assert_eq!(resolved.project.as_deref(), Some("referenced-project"));
    assert_eq!(resolved.location.as_deref(), Some("referenced-location"));
}

#[test]
fn a_missing_vertex_os_environ_reference_is_an_error() {
    let error = load(
        "litellm_settings:\n  vertex_project: os.environ/ABSENT_REFERENCE\n",
        &[],
        None,
    )
    .unwrap_err();
    let ConfigError::MissingEnvironment { variable, path } = error else {
        panic!("expected a missing environment variable");
    };
    assert_eq!(variable, "ABSENT_REFERENCE");
    assert_eq!(path, "litellm_settings.vertex_project");
}

#[test]
fn loads_vertex_adc_environment() {
    let absent = load("", &[], None).unwrap().adc;
    assert!(absent.application_credentials.is_none());
    assert!(absent.project.is_none());
    assert!(absent.gcloud_project.is_none());
    assert!(absent.gcloud_credentials.is_none());
    assert!(absent.gcloud_configuration.is_none());
    assert!(absent.metadata_host.is_none());

    let environment = load(
        "",
        &[
            ("GOOGLE_APPLICATION_CREDENTIALS", "/run/adc.json"),
            ("GOOGLE_CLOUD_PROJECT", "cloud-project"),
            ("GCLOUD_PROJECT", "legacy-project"),
            ("CLOUDSDK_CORE_PROJECT", "sdk-project"),
            ("GCE_METADATA_HOST", "metadata.example:8080"),
            ("GCE_METADATA_ROOT", "root.example"),
        ],
        Some("/home/tester"),
    )
    .unwrap()
    .adc;
    assert_eq!(
        environment.application_credentials,
        Some(PathBuf::from("/run/adc.json"))
    );
    assert_eq!(environment.project.as_deref(), Some("cloud-project"));
    assert_eq!(environment.gcloud_project.as_deref(), Some("sdk-project"));
    assert_eq!(
        environment.metadata_host.as_deref(),
        Some("metadata.example:8080")
    );
}

#[test]
fn resolves_the_gcloud_adc_file_paths() {
    let home = load("", &[], Some("/home/tester")).unwrap().adc;
    assert_eq!(
        home.gcloud_credentials,
        Some(PathBuf::from(
            "/home/tester/.config/gcloud/application_default_credentials.json"
        ))
    );
    assert_eq!(
        home.gcloud_configuration,
        Some(PathBuf::from(
            "/home/tester/.config/gcloud/configurations/config_default"
        ))
    );

    let configured = load("", &[("CLOUDSDK_CONFIG", "/sdk")], Some("/home/tester"))
        .unwrap()
        .adc;
    assert_eq!(
        configured.gcloud_credentials,
        Some(PathBuf::from("/sdk/application_default_credentials.json"))
    );
    assert_eq!(
        configured.gcloud_configuration,
        Some(PathBuf::from("/sdk/configurations/config_default"))
    );

    let active = load(
        "",
        &[
            ("CLOUDSDK_CONFIG", "/sdk"),
            ("CLOUDSDK_ACTIVE_CONFIG_NAME", "work"),
        ],
        None,
    )
    .unwrap()
    .adc;
    assert_eq!(
        active.gcloud_configuration,
        Some(PathBuf::from("/sdk/configurations/config_work"))
    );
}

#[test]
fn loads_the_legacy_vertex_adc_fallbacks() {
    let legacy_project = load("", &[("GCLOUD_PROJECT", "legacy-project")], None)
        .unwrap()
        .adc;
    assert_eq!(legacy_project.project.as_deref(), Some("legacy-project"));

    let root = load("", &[("GCE_METADATA_ROOT", "root.example")], None)
        .unwrap()
        .adc;
    assert_eq!(root.metadata_host.as_deref(), Some("root.example"));
}

#[test]
fn reads_an_empty_vertex_adc_environment_as_unset() {
    let empty = load(
        "",
        &[
            ("GOOGLE_APPLICATION_CREDENTIALS", ""),
            ("GOOGLE_CLOUD_PROJECT", ""),
            ("GCE_METADATA_HOST", ""),
        ],
        None,
    )
    .unwrap()
    .adc;
    assert!(empty.application_credentials.is_none());
    assert!(empty.project.is_none());
    assert!(empty.metadata_host.is_none());
}

#[test]
fn loads_service_account_vertex_credentials() {
    let service_account = r#"{"type":"service_account","project_id":"key-project","private_key":"KEYMATERIAL","client_email":"sa@example.iam.gserviceaccount.com"}"#;
    let loaded = load("", &[("VERTEXAI_CREDENTIALS", service_account)], None).unwrap();
    let Some(ConfiguredCredentials::Source(CredentialSource::ServiceAccount(account))) =
        loaded.credentials.as_ref()
    else {
        panic!("expected service account credentials");
    };
    assert_eq!(account.project_id.as_deref(), Some("key-project"));
    assert_eq!(account.client_email, "sa@example.iam.gserviceaccount.com");
    assert!(!format!("{loaded:?}").contains("KEYMATERIAL"));
}

#[test]
fn loads_authorized_user_vertex_credentials() {
    let authorized_user = r#"{"type":"authorized_user","client_id":"client","client_secret":"CLIENTSECRET","refresh_token":"REFRESHTOKEN","quota_project_id":"quota-project"}"#;
    let loaded = load("", &[("VERTEXAI_CREDENTIALS", authorized_user)], None).unwrap();
    let Some(ConfiguredCredentials::Source(CredentialSource::AuthorizedUser(user))) =
        loaded.credentials.as_ref()
    else {
        panic!("expected authorized user credentials");
    };
    assert_eq!(user.quota_project_id.as_deref(), Some("quota-project"));
    let debug = format!("{loaded:?}");
    assert!(!debug.contains("CLIENTSECRET"));
    assert!(!debug.contains("REFRESHTOKEN"));
}

#[test]
fn loads_external_account_vertex_credentials() {
    let external = r#"{"type":"external_account","audience":"audience","subject_token_type":"urn:ietf:params:oauth:token-type:jwt","credential_source":{"file":"/run/token"},"aws_region_name":"us-east-1"}"#;
    let loaded = load("", &[("VERTEXAI_CREDENTIALS", external)], None).unwrap();
    let Some(ConfiguredCredentials::Source(CredentialSource::ExternalAccount(account))) =
        loaded.credentials.as_ref()
    else {
        panic!("expected external account credentials");
    };
    assert_eq!(account.token_url, "https://sts.googleapis.com/v1/token");
    assert!(account.aws.any());
    assert!(matches!(
        &account.credential_source,
        ExternalCredentialSource::File { .. }
    ));
}

#[test]
fn loads_a_vertex_credentials_file_name() {
    let path = load(
        "",
        &[("VERTEXAI_CREDENTIALS", "/run/vertex-credentials.json")],
        None,
    )
    .unwrap();
    assert_eq!(
        path.credentials
            .as_ref()
            .and_then(ConfiguredCredentials::as_text),
        Some("/run/vertex-credentials.json")
    );
}

#[test]
fn refuses_executable_vertex_credentials() {
    let executable = r#"{"type":"external_account","audience":"audience","subject_token_type":"urn:ietf:params:oauth:token-type:jwt","credential_source":{"executable":{"command":"gcloud"}}}"#;
    let error = load("", &[("VERTEXAI_CREDENTIALS", executable)], None).unwrap_err();
    assert!(error.to_string().contains("vertex_credentials"), "{error}");
}

#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::azure_default_chain::{AzureCredentialType, Settings};

#[derive(Clone, Copy)]
enum Field {
    AzureCredential,
    FederatedTokenFile,
    CertificatePath,
    CertificatePassword,
    TokenCredentials,
    ClientCertificatePath,
    ClientCertificatePassword,
    ClientSendCertificateChain,
}

fn settings(variables: &[(&str, &str)]) -> Settings {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load("")
        .unwrap()
        .sections
        .azure_default_chain
}

fn read(settings: &Settings, field: Field) -> Option<String> {
    match field {
        Field::AzureCredential => settings
            .azure_credential
            .map(|credential| credential.as_str().to_owned()),
        Field::FederatedTokenFile => settings.azure_federated_token_file.clone(),
        Field::CertificatePath => settings.azure_certificate_path.clone(),
        Field::CertificatePassword => settings
            .azure_certificate_password
            .as_ref()
            .map(|secret| secret.expose_secret().to_owned()),
        Field::TokenCredentials => Some(settings.azure_token_credentials.clone()),
        Field::ClientCertificatePath => settings.azure_client_certificate_path.clone(),
        Field::ClientCertificatePassword => settings
            .client_certificate_password
            .as_ref()
            .map(|secret| secret.expose_secret().to_owned()),
        Field::ClientSendCertificateChain => {
            Some(settings.client_send_certificate_chain.to_string())
        }
    }
}

enum Scenario {
    Load(Case),
    UnrelatedKey,
}

struct Case {
    name: &'static str,
    value: &'static str,
    expected: &'static str,
    unset: Option<&'static str>,
    empty: Option<&'static str>,
    field: Field,
    secret: bool,
}

#[rstest]
#[case::loads_azure_credential(Scenario::Load(Case {
    name: "AZURE_CREDENTIAL",
    value: "CertificateCredential",
    expected: "CertificateCredential",
    unset: None,
    empty: None,
    field: Field::AzureCredential,
    secret: false,
}))]
#[case::loads_azure_federated_token_file(Scenario::Load(Case {
    name: "AZURE_FEDERATED_TOKEN_FILE",
    value: "/var/run/secrets/token",
    expected: "/var/run/secrets/token",
    unset: None,
    empty: Some(""),
    field: Field::FederatedTokenFile,
    secret: false,
}))]
#[case::loads_azure_certificate_path(Scenario::Load(Case {
    name: "AZURE_CERTIFICATE_PATH",
    value: "/certs/azure.pem",
    expected: "/certs/azure.pem",
    unset: None,
    empty: Some(""),
    field: Field::CertificatePath,
    secret: false,
}))]
#[case::loads_azure_certificate_password(Scenario::Load(Case {
    name: "AZURE_CERTIFICATE_PASSWORD",
    value: "fixture-certificate-password",
    expected: "fixture-certificate-password",
    unset: None,
    empty: Some(""),
    field: Field::CertificatePassword,
    secret: true,
}))]
#[case::loads_azure_token_credentials(Scenario::Load(Case {
    name: "AZURE_TOKEN_CREDENTIALS",
    value: " EnvironmentCredential ",
    expected: "environmentcredential",
    unset: Some(""),
    empty: Some(""),
    field: Field::TokenCredentials,
    secret: false,
}))]
#[case::loads_azure_client_certificate_path(Scenario::Load(Case {
    name: "AZURE_CLIENT_CERTIFICATE_PATH",
    value: "/certs/client.pem",
    expected: "/certs/client.pem",
    unset: None,
    empty: Some(""),
    field: Field::ClientCertificatePath,
    secret: false,
}))]
#[case::loads_azure_client_certificate_password(Scenario::Load(Case {
    name: "AZURE_CLIENT_CERTIFICATE_PASSWORD",
    value: "fixture-client-password",
    expected: "fixture-client-password",
    unset: None,
    empty: Some(""),
    field: Field::ClientCertificatePassword,
    secret: true,
}))]
#[case::loads_azure_client_send_certificate_chain(Scenario::Load(Case {
    name: "AZURE_CLIENT_SEND_CERTIFICATE_CHAIN",
    value: "false",
    expected: "true",
    unset: Some("false"),
    empty: Some("false"),
    field: Field::ClientSendCertificateChain,
    secret: false,
}))]
#[case::rejects_unrelated_key(Scenario::UnrelatedKey)]
fn azure_default_chain_config_cases(#[case] scenario: Scenario) {
    let case = match scenario {
        Scenario::Load(case) => case,
        Scenario::UnrelatedKey => {
            rejects_unrelated_key();
            return;
        }
    };
    let explicit = settings(&[(case.name, case.value)]);
    assert_eq!(read(&explicit, case.field).as_deref(), Some(case.expected));
    assert_eq!(read(&settings(&[]), case.field).as_deref(), case.unset);
    assert_eq!(
        read(&settings(&[(case.name, "")]), case.field).as_deref(),
        case.empty
    );
    assert_eq!(
        read(&settings(&[("UNRELATED", case.value)]), case.field).as_deref(),
        case.unset
    );
    if case.secret {
        assert!(!format!("{explicit:?}").contains(case.value));
        assert_eq!(
            explicit.redact_known(&format!("echo {} back", case.value)),
            "echo [redacted] back"
        );
    }
}

#[rstest]
#[case::client_secret("ClientSecretCredential", AzureCredentialType::ClientSecretCredential)]
#[case::certificate("CertificateCredential", AzureCredentialType::CertificateCredential)]
#[case::username_password(
    "UsernamePasswordCredential",
    AzureCredentialType::UsernamePasswordCredential
)]
#[case::managed_identity(
    "ManagedIdentityCredential",
    AzureCredentialType::ManagedIdentityCredential
)]
#[case::workload_identity(
    "WorkloadIdentityCredential",
    AzureCredentialType::WorkloadIdentityCredential
)]
#[case::default_credential("DefaultAzureCredential", AzureCredentialType::DefaultAzureCredential)]
#[case::deployment_identity(
    "DeploymentIdentityCredential",
    AzureCredentialType::DeploymentIdentityCredential
)]
fn loads_every_azure_credential_name(#[case] name: &str, #[case] credential: AzureCredentialType) {
    assert_eq!(
        settings(&[("AZURE_CREDENTIAL", name)]).azure_credential,
        Some(credential)
    );
    assert_eq!(credential.as_str(), name);
    assert_eq!(AzureCredentialType::parse(name), Some(credential));
}

fn rejects_unrelated_key() {
    let source = "litellm_settings:\n  azure_default_chain_unrelated_key: 1\n";
    let error = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .unwrap_err();
    let ConfigError::Unsupported { paths } = error else {
        panic!("the unrelated key loaded without an unsupported-key error");
    };
    assert_eq!(
        paths,
        vec!["litellm_settings.azure_default_chain_unrelated_key".to_owned()]
    );
}

#[rstest]
#[case::unknown_class_name("AzureCliCredential")]
#[case::lowercase_name("clientsecretcredential")]
#[case::empty_suffix("ClientSecretCredentialExtra")]
fn rejects_credential_names_outside_the_enum(#[case] value: &str) {
    let error = ConfigLoader {
        lookup: BTreeMap::from([("AZURE_CREDENTIAL".to_owned(), value.to_owned())]),
    }
    .load("")
    .unwrap_err();
    let ConfigError::InvalidScalar { path, .. } = &error else {
        panic!("the credential name {value} loaded");
    };
    assert!(path.contains(value));
}

#[rstest]
#[case::dev("dev")]
#[case::azure_cli(" AzureCliCredential ")]
#[case::visual_studio_code("visualstudiocodecredential")]
fn rejects_developer_credential_filters(#[case] filter: &str) {
    let error = ConfigLoader {
        lookup: BTreeMap::from([("AZURE_TOKEN_CREDENTIALS".to_owned(), filter.to_owned())]),
    }
    .load("")
    .unwrap_err();
    match &error {
        ConfigError::InvalidScalar { path, .. } => {
            assert!(path.contains("AZURE_TOKEN_CREDENTIALS"));
        }
        ConfigError::Environment { .. }
        | ConfigError::Yaml { .. }
        | ConfigError::Unsupported { .. }
        | ConfigError::MissingEnvironment { .. }
        | ConfigError::InvalidFloat { .. }
        | ConfigError::InvalidInteger { .. } => {
            panic!("the developer filter {filter} loaded as {error:?}");
        }
    }
}

#[rstest]
#[case::unset("")]
#[case::prod("prod")]
#[case::environment("environmentcredential")]
#[case::workload("workloadidentitycredential")]
#[case::managed("managedidentitycredential")]
#[case::unknown_name("not-a-credential")]
fn accepts_supported_credential_filters(#[case] filter: &str) {
    settings(&[("AZURE_TOKEN_CREDENTIALS", filter)]);
}

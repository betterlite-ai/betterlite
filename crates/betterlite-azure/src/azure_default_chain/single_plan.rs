use betterlite_config::sections::azure_default_chain::AzureCredentialType;
use secrecy::ExposeSecret as _;

use super::environment::ChainEnvironment;
use super::error::{DefaultChainError, workload_missing_message};
use super::grant::{
    CertificateRecord, ChainPlacement, ClientSecretRecord, Grants, ManagedIdentityRecord,
    WorkloadRecord,
};
use super::plan::{construction, missing_workload_arguments, workload_record};

// the explicit LiteLLM branch reads each variable with a required lookup, in the order it names them
fn required(value: Option<&str>, variable: &'static str) -> Result<String, DefaultChainError> {
    value.map_or_else(
        || Err(DefaultChainError::MissingEnvironment { variable }),
        |text| Ok(text.to_owned()),
    )
}

fn certificate_record(
    environment: &ChainEnvironment,
    scope: &str,
) -> Result<CertificateRecord, DefaultChainError> {
    let client_id = required(environment.client_id.as_deref(), "AZURE_CLIENT_ID")?;
    let tenant_id = required(environment.tenant_id.as_deref(), "AZURE_TENANT_ID")?;
    let certificate_path = required(
        environment.certificate_path.as_deref(),
        "AZURE_CERTIFICATE_PATH",
    )?;
    // the LiteLLM branch forwards this password only when it is truthy
    let password = environment
        .certificate_password
        .as_ref()
        .filter(|secret| !secret.expose_secret().is_empty())
        .cloned();
    Ok(CertificateRecord {
        client_id,
        tenant_id,
        certificate_path,
        password,
        send_certificate_chain: false,
        scope: scope.to_owned(),
    })
}

fn workload_single_record(
    environment: &ChainEnvironment,
    scope: &str,
) -> Result<WorkloadRecord, DefaultChainError> {
    workload_record(environment, scope).ok_or_else(|| DefaultChainError::InvalidFilter {
        message: workload_missing_message(&missing_workload_arguments(environment)),
    })
}

enum SingleRecord {
    Secret(ClientSecretRecord),
    Managed(ManagedIdentityRecord),
    Certificate(CertificateRecord),
    Workload(WorkloadRecord),
    MissingUsernamePassword(Vec<&'static str>),
    Unsupported(String),
}

fn single_record(
    credential: AzureCredentialType,
    environment: &ChainEnvironment,
    scope: &str,
) -> Result<SingleRecord, DefaultChainError> {
    Ok(match credential {
        AzureCredentialType::ClientSecretCredential => SingleRecord::Secret(ClientSecretRecord {
            client_id: required(environment.client_id.as_deref(), "AZURE_CLIENT_ID")?,
            client_secret: environment.client_secret.clone().ok_or(
                DefaultChainError::MissingEnvironment {
                    variable: "AZURE_CLIENT_SECRET",
                },
            )?,
            tenant_id: required(environment.tenant_id.as_deref(), "AZURE_TENANT_ID")?,
            scope: scope.to_owned(),
        }),
        AzureCredentialType::ManagedIdentityCredential => {
            SingleRecord::Managed(ManagedIdentityRecord {
                client_id: Some(required(
                    environment.client_id.as_deref(),
                    "AZURE_CLIENT_ID",
                )?),
                exclude_workload_identity: false,
                imds_probe: None,
                placement: ChainPlacement::Standalone,
                scope: scope.to_owned(),
            })
        }
        AzureCredentialType::CertificateCredential => {
            SingleRecord::Certificate(certificate_record(environment, scope)?)
        }
        AzureCredentialType::WorkloadIdentityCredential => {
            SingleRecord::Workload(workload_single_record(environment, scope)?)
        }
        AzureCredentialType::UsernamePasswordCredential => {
            SingleRecord::MissingUsernamePassword(vec!["client_id", "username", "password"])
        }
        AzureCredentialType::DefaultAzureCredential
        | AzureCredentialType::DeploymentIdentityCredential => {
            SingleRecord::Unsupported(credential.as_str().to_owned())
        }
    })
}

pub fn single_grant<'grants, F>(
    credential: AzureCredentialType,
    environment: &ChainEnvironment,
    grants: &'grants F,
    scope: &str,
) -> Result<F::Grant<'grants>, DefaultChainError>
where
    F: Grants + 'grants,
{
    match single_record(credential, environment, scope)? {
        SingleRecord::Secret(record) => grants.client_secret(&record).map_err(construction),
        SingleRecord::Managed(record) => grants.managed_identity(&record).map_err(construction),
        SingleRecord::Certificate(record) => grants.certificate(&record).map_err(construction),
        SingleRecord::Workload(record) => grants.workload(&record).map_err(construction),
        SingleRecord::MissingUsernamePassword(arguments) => {
            Err(DefaultChainError::MissingArguments {
                credential: "UsernamePasswordCredential.__init__()",
                arguments,
                details: "'client_id', 'username', and 'password'".to_owned(),
            })
        }
        SingleRecord::Unsupported(value) => Err(DefaultChainError::UnsupportedFilter { value }),
    }
}

use betterlite_config::sections::azure_default_chain::AzureCredentialType;
use secrecy::ExposeSecret as _;

use super::environment::ChainEnvironment;
use super::error::{DefaultChainError, workload_missing_message};
use super::grant::{
    CertificateRecord, ChainPlacement, ClientSecretRecord, Grants, ManagedIdentityRecord,
};
use super::plan::{construction, missing_workload_arguments, workload_record};

// the explicit LiteLLM branch reads each variable with a required lookup, in the order it names them
fn required(value: &Option<String>, variable: &'static str) -> Result<String, DefaultChainError> {
    match value {
        Some(value) => Ok(value.clone()),
        None => Err(DefaultChainError::MissingEnvironment { variable }),
    }
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
    match credential {
        AzureCredentialType::ClientSecretCredential => {
            let record = ClientSecretRecord {
                client_id: required(&environment.client_id, "AZURE_CLIENT_ID")?,
                client_secret: environment.client_secret.clone().ok_or(
                    DefaultChainError::MissingEnvironment {
                        variable: "AZURE_CLIENT_SECRET",
                    },
                )?,
                tenant_id: required(&environment.tenant_id, "AZURE_TENANT_ID")?,
                scope: scope.to_owned(),
            };
            grants
                .client_secret(&record)
                .map_err(|error| construction("ClientSecretCredential", error))
        }
        AzureCredentialType::ManagedIdentityCredential => {
            let record = ManagedIdentityRecord {
                client_id: Some(required(&environment.client_id, "AZURE_CLIENT_ID")?),
                exclude_workload_identity: false,
                imds_probe: None,
                placement: ChainPlacement::Standalone,
                scope: scope.to_owned(),
            };
            grants
                .managed_identity(&record)
                .map_err(|error| construction("ManagedIdentityCredential", error))
        }
        AzureCredentialType::CertificateCredential => {
            // the LiteLLM branch forwards this password only when it is truthy
            let password = environment
                .certificate_password
                .as_ref()
                .filter(|secret| !secret.expose_secret().is_empty())
                .cloned();
            let record = CertificateRecord {
                client_id: required(&environment.client_id, "AZURE_CLIENT_ID")?,
                tenant_id: required(&environment.tenant_id, "AZURE_TENANT_ID")?,
                certificate_path: required(
                    &environment.certificate_path,
                    "AZURE_CERTIFICATE_PATH",
                )?,
                password,
                send_certificate_chain: false,
                scope: scope.to_owned(),
            };
            grants
                .certificate(&record)
                .map_err(|error| construction("CertificateCredential", error))
        }
        AzureCredentialType::WorkloadIdentityCredential => {
            let record = workload_record(environment, scope).ok_or_else(|| {
                DefaultChainError::InvalidFilter {
                    message: workload_missing_message(&missing_workload_arguments(environment)),
                }
            })?;
            grants
                .workload(&record)
                .map_err(|error| construction("WorkloadIdentityCredential", error))
        }
        AzureCredentialType::UsernamePasswordCredential => {
            Err(DefaultChainError::MissingArguments {
                credential: "UsernamePasswordCredential.__init__()",
                arguments: vec!["client_id", "username", "password"],
                details: "'client_id', 'username', and 'password'".to_owned(),
            })
        }
        AzureCredentialType::DefaultAzureCredential
        | AzureCredentialType::DeploymentIdentityCredential => {
            Err(DefaultChainError::UnsupportedFilter {
                value: credential.as_str().to_owned(),
            })
        }
    }
}

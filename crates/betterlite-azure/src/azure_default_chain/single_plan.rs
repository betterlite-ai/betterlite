use betterlite_config::sections::azure_default_chain::AzureCredentialType;
use secrecy::ExposeSecret as _;

use super::environment::ChainEnvironment;
use super::error::{DefaultChainError, workload_missing_message};
use super::grant::{
    CertificateRecord, ChainPlacement, ClientSecretRecord, Grants, ManagedIdentityRecord,
};
use super::plan::{construction, missing_workload_arguments, required, workload_record};

fn single_client_secret<'grants, F>(
    environment: &ChainEnvironment,
    grants: &'grants F,
    scope: &str,
) -> Result<F::Grant<'grants>, DefaultChainError>
where
    F: Grants + 'grants,
{
    let record = ClientSecretRecord {
        client_id: required(environment.client_id.as_deref(), "AZURE_CLIENT_ID")?.to_owned(),
        client_secret: environment.client_secret.clone().ok_or(
            DefaultChainError::MissingEnvironment {
                variable: "AZURE_CLIENT_SECRET",
            },
        )?,
        tenant_id: required(environment.tenant_id.as_deref(), "AZURE_TENANT_ID")?.to_owned(),
        scope: scope.to_owned(),
    };
    grants
        .client_secret(&record)
        .map_err(|error| construction("ClientSecretCredential", error))
}

fn single_managed_identity<'grants, F>(
    environment: &ChainEnvironment,
    grants: &'grants F,
    scope: &str,
) -> Result<F::Grant<'grants>, DefaultChainError>
where
    F: Grants + 'grants,
{
    let record = ManagedIdentityRecord {
        client_id: Some(required(environment.client_id.as_deref(), "AZURE_CLIENT_ID")?.to_owned()),
        exclude_workload_identity: false,
        imds_probe: None,
        placement: ChainPlacement::Standalone,
        scope: scope.to_owned(),
    };
    grants
        .managed_identity(&record)
        .map_err(|error| construction("ManagedIdentityCredential", error))
}

fn single_certificate<'grants, F>(
    environment: &ChainEnvironment,
    grants: &'grants F,
    scope: &str,
) -> Result<F::Grant<'grants>, DefaultChainError>
where
    F: Grants + 'grants,
{
    // the LiteLLM branch forwards this password only when it is truthy
    let password = environment
        .certificate_password
        .as_ref()
        .filter(|secret| !secret.expose_secret().is_empty())
        .cloned();
    let record = CertificateRecord {
        client_id: required(environment.client_id.as_deref(), "AZURE_CLIENT_ID")?.to_owned(),
        tenant_id: required(environment.tenant_id.as_deref(), "AZURE_TENANT_ID")?.to_owned(),
        certificate_path: required(
            environment.certificate_path.as_deref(),
            "AZURE_CERTIFICATE_PATH",
        )?
        .to_owned(),
        password,
        send_certificate_chain: false,
        scope: scope.to_owned(),
    };
    grants
        .certificate(&record)
        .map_err(|error| construction("CertificateCredential", error))
}

fn single_workload<'grants, F>(
    environment: &ChainEnvironment,
    grants: &'grants F,
    scope: &str,
) -> Result<F::Grant<'grants>, DefaultChainError>
where
    F: Grants + 'grants,
{
    let record =
        workload_record(environment, scope).ok_or_else(|| DefaultChainError::InvalidFilter {
            message: workload_missing_message(&missing_workload_arguments(environment)),
        })?;
    grants
        .workload(&record)
        .map_err(|error| construction("WorkloadIdentityCredential", error))
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
            single_client_secret(environment, grants, scope)
        }
        AzureCredentialType::ManagedIdentityCredential => {
            single_managed_identity(environment, grants, scope)
        }
        AzureCredentialType::CertificateCredential => {
            single_certificate(environment, grants, scope)
        }
        AzureCredentialType::WorkloadIdentityCredential => {
            single_workload(environment, grants, scope)
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

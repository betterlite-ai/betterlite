use secrecy::ExposeSecret as _;

use crate::azure_hosted_identity::environment::HostedEnvironment;

use super::environment::ManagedIdentityEnvironment;
use super::error::ManagedIdentityError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagedIdentityKind {
    ServiceFabric,
    AppService,
    Arc,
    AzureMl,
    CloudShell,
    Workload,
    Imds,
    Unconfigured,
}

impl ManagedIdentityKind {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::ServiceFabric => "service_fabric",
            Self::AppService => "app_service",
            Self::Arc => "arc",
            Self::AzureMl => "azure_ml",
            Self::CloudShell => "cloud_shell",
            Self::Workload => "workload",
            Self::Imds => "imds",
            Self::Unconfigured => "none",
        }
    }

    // MSAL re-reads the environment by key presence once a managed identity client is selected
    #[must_use]
    pub const fn is_msal_backed(self) -> bool {
        matches!(
            self,
            Self::ServiceFabric | Self::AppService | Self::Arc | Self::AzureMl | Self::Imds
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkloadSelection {
    pub client_id: String,
    pub tenant_id: String,
    pub token_file: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub kind: ManagedIdentityKind,
    pub client_id: Option<String>,
    pub workload: Option<WorkloadSelection>,
}

fn user_assigned(client_id: Option<&str>) -> Option<String> {
    client_id
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

// the SDK's outer selector tests truthiness, unlike the presence test MSAL runs later
fn hosted_selection(hosted: &HostedEnvironment, identity: Option<&str>) -> Option<Selection> {
    let client_id = identity.map(str::to_owned);
    if hosted
        .identity_endpoint
        .as_deref()
        .is_some_and(|value| !value.is_empty())
    {
        let kind = identity_endpoint_kind(hosted);
        return Some(Selection {
            kind,
            client_id,
            workload: None,
        });
    }
    if hosted
        .msi_endpoint
        .as_deref()
        .is_some_and(|value| !value.is_empty())
    {
        let has_secret = hosted
            .msi_secret
            .as_ref()
            .is_some_and(|secret| !secret.expose_secret().is_empty());
        let kind = if has_secret {
            ManagedIdentityKind::AzureMl
        } else {
            ManagedIdentityKind::CloudShell
        };
        return Some(Selection {
            kind,
            client_id,
            workload: None,
        });
    }
    None
}

fn identity_endpoint_kind(hosted: &HostedEnvironment) -> ManagedIdentityKind {
    let has_header = hosted
        .identity_header
        .as_ref()
        .is_some_and(|secret| !secret.expose_secret().is_empty());
    if has_header {
        if hosted
            .identity_server_thumbprint
            .as_deref()
            .is_some_and(|value| !value.is_empty())
        {
            ManagedIdentityKind::ServiceFabric
        } else {
            ManagedIdentityKind::AppService
        }
    } else if hosted
        .imds_endpoint
        .as_deref()
        .is_some_and(|value| !value.is_empty())
    {
        ManagedIdentityKind::Arc
    } else {
        // a truthy IDENTITY_ENDPOINT without its partners selects no credential at all
        ManagedIdentityKind::Unconfigured
    }
}

fn workload_variables(environment: &ManagedIdentityEnvironment) -> Option<(&str, &str)> {
    let authority = environment.azure_authority_host.as_deref()?;
    let tenant_id = environment.azure_tenant_id.as_deref()?;
    let token_file = environment.azure_federated_token_file.as_deref()?;
    if authority.is_empty() || tenant_id.is_empty() || token_file.is_empty() {
        return None;
    }
    Some((tenant_id, token_file))
}

pub fn select(
    environment: &ManagedIdentityEnvironment,
    client_id: Option<&str>,
    exclude_workload_identity: bool,
) -> Result<Selection, ManagedIdentityError> {
    let identity = user_assigned(client_id);
    if let Some(selection) = hosted_selection(&environment.hosted, identity.as_deref()) {
        return Ok(selection);
    }
    if !exclude_workload_identity
        && let Some((tenant_id, token_file)) = workload_variables(environment)
    {
        let workload_client_id = identity.or_else(|| {
            environment
                .azure_client_id
                .clone()
                .filter(|value| !value.is_empty())
        });
        let Some(workload_id) = workload_client_id else {
            return Err(ManagedIdentityError::WorkloadClientId);
        };
        return Ok(Selection {
            kind: ManagedIdentityKind::Workload,
            client_id: Some(workload_id.clone()),
            workload: Some(WorkloadSelection {
                client_id: workload_id,
                tenant_id: tenant_id.to_owned(),
                token_file: token_file.to_owned(),
            }),
        });
    }
    Ok(Selection {
        kind: ManagedIdentityKind::Imds,
        client_id: identity,
        workload: None,
    })
}

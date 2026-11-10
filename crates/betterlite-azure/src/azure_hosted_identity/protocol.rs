use crate::azure_arc_identity::{ArcHost, select_arc_endpoint};

use super::environment::HostedEnvironment;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostedProtocol {
    ServiceFabric,
    AppService,
    AzureMl,
    Arc,
    Vm,
}

impl HostedProtocol {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::ServiceFabric => "service_fabric",
            Self::AppService => "app_service",
            Self::AzureMl => "azure_ml",
            Self::Arc => "arc",
            Self::Vm => "vm",
        }
    }
}

// MSAL tests environment-key presence here, while the outer credential selector tests truthiness
#[must_use]
pub fn select_hosted_protocol<H>(environment: &HostedEnvironment, arc: &H) -> HostedProtocol
where
    H: ArcHost,
{
    let identity_endpoint = environment.identity_endpoint.as_deref();
    if identity_endpoint.is_some()
        && environment.identity_header.is_some()
        && environment.identity_server_thumbprint.is_some()
    {
        return HostedProtocol::ServiceFabric;
    }
    if identity_endpoint.is_some() && environment.identity_header.is_some() {
        return HostedProtocol::AppService;
    }
    if environment.msi_endpoint.is_some() && environment.msi_secret.is_some() {
        return HostedProtocol::AzureMl;
    }
    if identity_endpoint.is_some() && environment.imds_endpoint.is_some() {
        return if identity_endpoint.is_some_and(|endpoint| !endpoint.is_empty()) {
            HostedProtocol::Arc
        } else {
            HostedProtocol::Vm
        };
    }
    if select_arc_endpoint(arc).is_some() {
        HostedProtocol::Arc
    } else {
        HostedProtocol::Vm
    }
}

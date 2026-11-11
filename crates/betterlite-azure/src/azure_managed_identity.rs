pub mod decode;
pub mod environment;
pub mod error;
pub mod imds;
pub mod retry;
pub mod runtime;
pub mod selection;
pub mod workload;

mod tests;

use std::collections::BTreeMap;
use std::io::Result as IoResult;

use secrecy::SecretString;

use crate::azure_arc_identity::{
    ArcHost, ArcIdentity, ArcRequest, ArcResponse, ArcTransport, acquire_arc_token,
    select_arc_endpoint,
};
use crate::azure_client_secret::token::AccessToken;
use crate::azure_hosted_identity::credential::{
    CloudShellCredential, HostedCredential, HostedIdentity,
};
use crate::azure_hosted_identity::msal_reply::MsalKind;
use crate::azure_hosted_identity::protocol::{HostedProtocol, select_hosted_protocol};

use environment::ManagedIdentityEnvironment;
use error::ManagedIdentityError;
use imds::ProbePolicy;
use runtime::ManagedIdentityIo;
use selection::{ManagedIdentityKind, Selection};

#[derive(Debug)]
pub struct ManagedIdentityToken {
    pub token: SecretString,
    pub expires_on: Option<i64>,
    pub token_type: Option<String>,
    pub refresh_on: Option<i64>,
}

impl From<AccessToken> for ManagedIdentityToken {
    fn from(token: AccessToken) -> Self {
        Self {
            token: token.token,
            expires_on: Some(token.expires_on),
            token_type: Some(token.token_type),
            refresh_on: token.refresh_on,
        }
    }
}

// a fixture transport reaches the arc protocol behind a trait object; the generic seam needs the forwarding impl
impl ArcTransport for &dyn ArcTransport {
    fn get(&self, request: &ArcRequest) -> IoResult<ArcResponse> {
        (*self).get(request)
    }
}

pub struct ManagedIdentityCredential<'environment, H: ArcHost> {
    pub environment: &'environment ManagedIdentityEnvironment,
    pub host: &'environment H,
    pub client_id: Option<&'environment str>,
    pub exclude_workload_identity: bool,
    pub within_default_credential: bool,
    pub probe: ProbePolicy,
}

impl<H: ArcHost> ManagedIdentityCredential<'_, H> {
    pub fn select(&self) -> Result<Selection, ManagedIdentityError> {
        selection::select(
            self.environment,
            self.client_id,
            self.exclude_workload_identity,
        )
    }

    #[must_use]
    pub fn inner_protocol(&self, selection: &Selection) -> Option<HostedProtocol> {
        selection
            .kind
            .is_msal_backed()
            .then(|| select_hosted_protocol(&self.environment.hosted, self.host))
    }

    pub fn token(
        &self,
        io: &ManagedIdentityIo<'_>,
        scopes: &[&str],
    ) -> Result<ManagedIdentityToken, ManagedIdentityError> {
        let [scope] = scopes else {
            return Err(ManagedIdentityError::ScopeCount);
        };
        let selection = self.select()?;
        match selection.kind {
            ManagedIdentityKind::Unconfigured => Err(ManagedIdentityError::NoEndpoint),
            ManagedIdentityKind::Workload => {
                let workload = selection
                    .workload
                    .as_ref()
                    .ok_or(ManagedIdentityError::WorkloadClientId)?;
                let token =
                    workload::acquire(io.workload.exchange, io.workload.files, workload, scope)?;
                Ok(ManagedIdentityToken {
                    token,
                    expires_on: None,
                    token_type: None,
                    refresh_on: None,
                })
            }
            ManagedIdentityKind::CloudShell => {
                let resource = scope.strip_suffix("/.default").unwrap_or(scope);
                CloudShellCredential {
                    environment: self.environment.hosted.clone(),
                    identity: self.hosted_identity(),
                    within_default_credential: self.within_default_credential,
                }
                .acquire(&io.hosted, resource)
                .map(ManagedIdentityToken::from)
                .map_err(ManagedIdentityError::from)
            }
            ManagedIdentityKind::ServiceFabric
            | ManagedIdentityKind::AppService
            | ManagedIdentityKind::Arc
            | ManagedIdentityKind::AzureMl
            | ManagedIdentityKind::Imds => self.msal_backed(io, scope),
        }
    }

    fn hosted_identity(&self) -> HostedIdentity {
        HostedIdentity {
            client_id: self.client_id.map(str::to_owned),
            identity_config: BTreeMap::new(),
        }
    }

    // once an MSAL-backed credential is selected, MSAL re-reads the environment by key presence
    fn msal_backed(
        &self,
        io: &ManagedIdentityIo<'_>,
        scope: &str,
    ) -> Result<ManagedIdentityToken, ManagedIdentityError> {
        let resource = scope.strip_suffix("/.default").unwrap_or(scope);
        let kind = match select_hosted_protocol(&self.environment.hosted, self.host) {
            HostedProtocol::ServiceFabric => MsalKind::ServiceFabric,
            HostedProtocol::AppService => MsalKind::AppService,
            HostedProtocol::AzureMl => MsalKind::AzureMl,
            HostedProtocol::Arc => return self.arc(io, resource),
            HostedProtocol::Vm => {
                return imds::acquire(
                    self.environment,
                    &io.imds,
                    resource,
                    self.client_id,
                    self.probe,
                )
                .map(ManagedIdentityToken::from);
            }
        };
        HostedCredential {
            environment: self.environment.hosted.clone(),
            identity: self.hosted_identity(),
        }
        .acquire(kind, &io.hosted, resource)
        .map(ManagedIdentityToken::from)
        .map_err(ManagedIdentityError::from)
    }

    fn arc(
        &self,
        io: &ManagedIdentityIo<'_>,
        resource: &str,
    ) -> Result<ManagedIdentityToken, ManagedIdentityError> {
        let endpoint = select_arc_endpoint(self.host).ok_or(ManagedIdentityError::ArcEndpoint)?;
        let identity = match self.client_id {
            Some(client_id) if !client_id.is_empty() => ArcIdentity::UserAssigned,
            Some(_) | None => ArcIdentity::SystemAssigned,
        };
        let token = acquire_arc_token(self.host, &io.arc, &endpoint, resource, identity)?;
        Ok(ManagedIdentityToken {
            token: token.token,
            expires_on: Some(token.expires_on),
            token_type: Some(token.token_type),
            refresh_on: token.refresh_on,
        })
    }
}

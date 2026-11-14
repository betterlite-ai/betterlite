mod arc;
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

use crate::azure_arc_identity::{ArcHost, ArcTransport, MetadataRequest, MetadataResponse};
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
    fn get(&self, request: &MetadataRequest) -> IoResult<MetadataResponse> {
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
        let resource = scope.strip_suffix("/.default").unwrap_or(scope);
        if selection.kind == ManagedIdentityKind::Unconfigured {
            return Err(ManagedIdentityError::NoEndpoint);
        }
        if selection.kind == ManagedIdentityKind::Workload {
            return Self::workload(io, &selection, scope);
        }
        if selection.kind == ManagedIdentityKind::CloudShell {
            let shell = CloudShellCredential {
                environment: self.environment.hosted.clone(),
                identity: HostedIdentity {
                    client_id: self.client_id.map(str::to_owned),
                    identity_config: BTreeMap::new(),
                },
                within_default_credential: self.within_default_credential,
            };
            return Ok(ManagedIdentityToken::from(
                shell.acquire(&io.hosted, resource)?,
            ));
        }
        self.msal_backed(io, resource)
    }

    fn workload(
        io: &ManagedIdentityIo<'_>,
        selection: &Selection,
        scope: &str,
    ) -> Result<ManagedIdentityToken, ManagedIdentityError> {
        let workload = selection
            .workload
            .as_ref()
            .ok_or(ManagedIdentityError::WorkloadClientId)?;
        let token = workload::acquire(io.workload.exchange, io.workload.files, workload, scope)?;
        Ok(ManagedIdentityToken {
            token,
            expires_on: None,
            token_type: None,
            refresh_on: None,
        })
    }

    // once an MSAL-backed credential is selected, MSAL re-reads the environment by key presence
    fn msal_backed(
        &self,
        io: &ManagedIdentityIo<'_>,
        resource: &str,
    ) -> Result<ManagedIdentityToken, ManagedIdentityError> {
        let protocol = select_hosted_protocol(&self.environment.hosted, self.host);
        match msal_kind(protocol) {
            Some(kind) => {
                let hosted = HostedCredential {
                    environment: self.environment.hosted.clone(),
                    identity: HostedIdentity {
                        client_id: self.client_id.map(str::to_owned),
                        identity_config: BTreeMap::new(),
                    },
                };
                Ok(ManagedIdentityToken::from(
                    hosted.acquire(kind, &io.hosted, resource)?,
                ))
            }
            None if protocol == HostedProtocol::Arc => {
                arc::acquire(self.host, io.arc, self.client_id, resource)
            }
            None => {
                let (env, probe) = (self.environment, self.probe);
                let token = imds::acquire(env, &io.imds, resource, self.client_id, probe)?;
                Ok(ManagedIdentityToken::from(token))
            }
        }
    }
}

const fn msal_kind(protocol: HostedProtocol) -> Option<MsalKind> {
    match protocol {
        HostedProtocol::ServiceFabric => Some(MsalKind::ServiceFabric),
        HostedProtocol::AppService => Some(MsalKind::AppService),
        HostedProtocol::AzureMl => Some(MsalKind::AzureMl),
        HostedProtocol::Arc | HostedProtocol::Vm => None,
    }
}

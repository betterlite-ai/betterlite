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

impl ManagedIdentityToken {
    const fn issued(
        token: SecretString,
        expires_on: i64,
        token_type: String,
        refresh_on: Option<i64>,
    ) -> Self {
        Self {
            token,
            expires_on: Some(expires_on),
            token_type: Some(token_type),
            refresh_on,
        }
    }
}

impl From<AccessToken> for ManagedIdentityToken {
    fn from(token: AccessToken) -> Self {
        Self::issued(
            token.token,
            token.expires_on,
            token.token_type,
            token.refresh_on,
        )
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
        let resource = scope.strip_suffix("/.default").unwrap_or(scope);
        match selection.kind {
            ManagedIdentityKind::Unconfigured => Err(ManagedIdentityError::NoEndpoint),
            ManagedIdentityKind::Workload => Self::workload(io, &selection, scope),
            ManagedIdentityKind::CloudShell => self.cloud_shell(io, resource),
            ManagedIdentityKind::ServiceFabric
            | ManagedIdentityKind::AppService
            | ManagedIdentityKind::Arc
            | ManagedIdentityKind::AzureMl
            | ManagedIdentityKind::Imds => self.msal_backed(io, resource),
        }
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

    fn cloud_shell(
        &self,
        io: &ManagedIdentityIo<'_>,
        resource: &str,
    ) -> Result<ManagedIdentityToken, ManagedIdentityError> {
        let shell = CloudShellCredential {
            environment: self.environment.hosted.clone(),
            identity: self.hosted_identity(),
            within_default_credential: self.within_default_credential,
        };
        Ok(ManagedIdentityToken::from(
            shell.acquire(&io.hosted, resource)?,
        ))
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
        resource: &str,
    ) -> Result<ManagedIdentityToken, ManagedIdentityError> {
        let hosted = HostedCredential {
            environment: self.environment.hosted.clone(),
            identity: self.hosted_identity(),
        };
        let token = match select_hosted_protocol(&self.environment.hosted, self.host) {
            HostedProtocol::ServiceFabric => {
                hosted.acquire(MsalKind::ServiceFabric, &io.hosted, resource)?
            }
            HostedProtocol::AppService => {
                hosted.acquire(MsalKind::AppService, &io.hosted, resource)?
            }
            HostedProtocol::AzureMl => hosted.acquire(MsalKind::AzureMl, &io.hosted, resource)?,
            HostedProtocol::Arc => return self.arc(io, resource),
            HostedProtocol::Vm => imds::acquire(
                self.environment,
                &io.imds,
                resource,
                self.client_id,
                self.probe,
            )?,
        };
        Ok(ManagedIdentityToken::from(token))
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
        Ok(ManagedIdentityToken::issued(
            token.token,
            token.expires_on,
            token.token_type,
            token.refresh_on,
        ))
    }
}

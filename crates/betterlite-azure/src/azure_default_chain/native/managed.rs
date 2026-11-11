use crate::azure_arc_identity::ArcHost;
use crate::azure_managed_identity::environment::ManagedIdentityEnvironment;
use crate::azure_managed_identity::error::ManagedIdentityError;
use crate::azure_managed_identity::imds::ProbePolicy;
use crate::azure_managed_identity::runtime::ManagedIdentityIo;
use crate::azure_managed_identity::{ManagedIdentityCredential, ManagedIdentityToken};

use super::super::grant::{ChainPlacement, ChainToken, Grant, LinkError};

pub struct ManagedIdentityGrant<'io, H: ArcHost> {
    pub environment: &'io ManagedIdentityEnvironment,
    pub host: &'io H,
    pub io: ManagedIdentityIo<'io>,
    pub client_id: Option<String>,
    pub exclude_workload_identity: bool,
    pub imds_probe: Option<bool>,
    pub placement: ChainPlacement,
    pub scope: String,
}

impl<H: ArcHost> Grant for ManagedIdentityGrant<'_, H> {
    fn token(&self) -> Result<ChainToken, LinkError> {
        let credential = ManagedIdentityCredential {
            environment: self.environment,
            host: self.host,
            client_id: self.client_id.as_deref(),
            exclude_workload_identity: self.exclude_workload_identity,
            within_default_credential: self.placement.within_default_credential(),
            probe: ProbePolicy {
                enable: self.imds_probe,
                within_credential_chain: self.placement.within_credential_chain(),
            },
        };
        let ManagedIdentityToken {
            token,
            expires_on,
            token_type,
            refresh_on,
        } = credential
            .token(&self.io, &[&self.scope])
            .map_err(|error| managed_error(&error))?;
        Ok(ChainToken {
            token,
            expires_on,
            token_type,
            refresh_on,
        })
    }
}

// the SDK raises CredentialUnavailableError for these and lets the chain continue; everything else stops it
fn managed_error(error: &ManagedIdentityError) -> LinkError {
    let message = error.to_string();
    match error {
        ManagedIdentityError::NoEndpoint
        | ManagedIdentityError::ImdsUnavailable
        | ManagedIdentityError::TokenRequest { .. }
        | ManagedIdentityError::ImdsError { .. }
        | ManagedIdentityError::NoIdentityAssigned
        | ManagedIdentityError::AuthenticationUnavailable => LinkError::Unavailable(message),
        ManagedIdentityError::ScopeCount
        | ManagedIdentityError::WorkloadClientId
        | ManagedIdentityError::ClientAuthentication { .. }
        | ManagedIdentityError::ArcEndpoint
        | ManagedIdentityError::ExpiryOverflow
        | ManagedIdentityError::TokenFile { .. }
        | ManagedIdentityError::TokenFileEncoding { .. }
        | ManagedIdentityError::ImdsReply(_)
        | ManagedIdentityError::Hosted(_)
        | ManagedIdentityError::Arc(_)
        | ManagedIdentityError::Oidc(_) => LinkError::Authentication(message),
    }
}

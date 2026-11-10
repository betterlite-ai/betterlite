use std::collections::BTreeMap;

use secrecy::SecretString;

use super::cloud_shell_reply;
use super::environment::HostedEnvironment;
use super::error::HostedIdentityError;
use super::msal_reply::{self, MsalKind};
use super::request::{
    APP_SERVICE_API_VERSION, AZURE_ML_API_VERSION, HostedIo, HostedRequest,
    SERVICE_FABRIC_API_VERSION, TokenResult, cloud_shell_request, token_request,
};

const CLOUD_SHELL_IDENTITY_KEYS: [&str; 3] = ["object_id", "resource_id", "client_id"];

#[derive(Debug, Clone, Default)]
pub struct HostedIdentity {
    pub client_id: Option<String>,
    pub identity_config: BTreeMap<String, String>,
}

impl HostedIdentity {
    fn user_client_id(&self) -> Option<&str> {
        self.client_id
            .as_deref()
            .filter(|client_id| !client_id.is_empty())
    }
}

#[derive(Debug)]
pub struct HostedCredential {
    environment: HostedEnvironment,
    identity: HostedIdentity,
}

impl HostedCredential {
    #[must_use]
    pub const fn new(environment: HostedEnvironment, identity: HostedIdentity) -> Self {
        Self {
            environment,
            identity,
        }
    }

    pub fn acquire(&self, kind: MsalKind, io: &HostedIo<'_>, resource: &str) -> TokenResult {
        let request = self.request(kind, resource)?;
        let (response, started) = io.exchange(&request)?;
        msal_reply::msal_response(kind, &response, &self.environment, started, io.clock)
    }

    fn request(
        &self,
        kind: MsalKind,
        resource: &str,
    ) -> Result<HostedRequest, HostedIdentityError> {
        let environment = &self.environment;
        let (endpoint, header_secret) = match kind {
            MsalKind::AppService | MsalKind::ServiceFabric => (
                environment.identity_endpoint.as_deref(),
                environment.identity_header.as_ref(),
            ),
            MsalKind::AzureMl => (
                environment.msi_endpoint.as_deref(),
                environment.msi_secret.as_ref(),
            ),
        };
        if matches!(kind, MsalKind::ServiceFabric)
            && (self.identity.user_client_id().is_some()
                || !self.identity.identity_config.is_empty())
        {
            return Err(HostedIdentityError::ServiceFabricIdentityConfig);
        }
        let (Some(url), Some(secret)) = (endpoint, header_secret) else {
            return Err(HostedIdentityError::MissingEnvironment { protocol: kind });
        };
        let (version, header, client_param) = match kind {
            MsalKind::AppService => (
                APP_SERVICE_API_VERSION,
                "X-IDENTITY-HEADER",
                Some("client_id"),
            ),
            MsalKind::AzureMl => (AZURE_ML_API_VERSION, "secret", Some("clientid")),
            MsalKind::ServiceFabric => (SERVICE_FABRIC_API_VERSION, "Secret", None),
        };
        let mut headers = vec![(header, secret.clone())];
        if matches!(kind, MsalKind::AppService) {
            headers.push(("Metadata", SecretString::from("true")));
        }
        Ok(token_request(
            url,
            version,
            resource,
            client_param.zip(self.identity.user_client_id()),
            headers,
        ))
    }
}

#[derive(Debug)]
pub struct CloudShellCredential {
    environment: HostedEnvironment,
    identity: HostedIdentity,
    within_default_credential: bool,
}

impl CloudShellCredential {
    #[must_use]
    pub const fn new(
        environment: HostedEnvironment,
        identity: HostedIdentity,
        within_default_credential: bool,
    ) -> Self {
        Self {
            environment,
            identity,
            within_default_credential,
        }
    }

    pub fn acquire(&self, io: &HostedIo<'_>, resource: &str) -> TokenResult {
        let request = self.request(resource)?;
        let (response, request_time) = io.exchange(&request)?;
        cloud_shell_reply::cloud_shell_response(&response, &self.environment, request_time)
    }

    fn request(&self, resource: &str) -> Result<HostedRequest, HostedIdentityError> {
        if !self.within_default_credential {
            self.reject_identity()?;
        }
        let endpoint = self
            .environment
            .msi_endpoint
            .as_deref()
            .filter(|endpoint| !endpoint.is_empty())
            .ok_or(HostedIdentityError::CloudShellUnavailable)?;
        Ok(cloud_shell_request(
            endpoint,
            resource,
            &self.identity.identity_config,
            self.identity.user_client_id(),
        ))
    }

    fn reject_identity(&self) -> Result<(), HostedIdentityError> {
        if self.identity.user_client_id().is_some() {
            return Err(HostedIdentityError::CloudShellClientId);
        }
        if self
            .identity
            .identity_config
            .keys()
            .any(|key| CLOUD_SHELL_IDENTITY_KEYS.contains(&key.as_str()))
        {
            return Err(HostedIdentityError::CloudShellIdentityConfig);
        }
        Ok(())
    }
}

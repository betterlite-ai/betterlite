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

#[derive(Debug, Clone, Copy)]
enum IdentityKind {
    Client,
    Resource,
    Object,
}

// MSAL's get_managed_identity reads identity_config in this order, after the client_id argument
const IDENTITY_CONFIG_KEYS: [(&str, IdentityKind); 3] = [
    ("client_id", IdentityKind::Client),
    ("resource_id", IdentityKind::Resource),
    ("object_id", IdentityKind::Object),
];

impl IdentityKind {
    const fn wire_name(self, protocol: MsalKind) -> &'static str {
        match (protocol, self) {
            (MsalKind::AzureMl, Self::Client) => "clientid",
            (MsalKind::AzureMl, Self::Resource) => "msi_res_id",
            (_, Self::Client) => "client_id",
            (_, Self::Resource) => "mi_res_id",
            (_, Self::Object) => "object_id",
        }
    }
}

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

    fn user_assigned(&self) -> Option<(IdentityKind, &str)> {
        if let Some(client_id) = self.user_client_id() {
            return Some((IdentityKind::Client, client_id));
        }
        IDENTITY_CONFIG_KEYS.iter().find_map(|(key, kind)| {
            self.identity_config
                .get(*key)
                .filter(|value| !value.is_empty())
                .map(|value| (*kind, value.as_str()))
        })
    }
}

#[derive(Debug)]
pub struct HostedCredential {
    pub environment: HostedEnvironment,
    pub identity: HostedIdentity,
}

impl HostedCredential {
    pub fn acquire(&self, kind: MsalKind, io: &HostedIo<'_>, resource: &str) -> TokenResult {
        let request = self.request(kind, resource)?;
        let (response, started) = io.exchange(&request)?;
        msal_reply::msal_response(kind, &response, &self.environment, started, io)
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
        let (version, header) = match kind {
            MsalKind::AppService => (APP_SERVICE_API_VERSION, "X-IDENTITY-HEADER"),
            MsalKind::AzureMl => (AZURE_ML_API_VERSION, "secret"),
            MsalKind::ServiceFabric => (SERVICE_FABRIC_API_VERSION, "Secret"),
        };
        let identity_param = self
            .identity
            .user_assigned()
            .map(|(identity, value)| (identity.wire_name(kind), value));
        let mut headers = vec![(header, secret.clone())];
        if matches!(kind, MsalKind::AppService) {
            headers.push(("Metadata", SecretString::from("true")));
        }
        Ok(token_request(
            url,
            version,
            resource,
            identity_param,
            headers,
        ))
    }
}

#[derive(Debug)]
pub struct CloudShellCredential {
    pub environment: HostedEnvironment,
    pub identity: HostedIdentity,
    pub within_default_credential: bool,
}

impl CloudShellCredential {
    pub fn acquire(&self, io: &HostedIo<'_>, resource: &str) -> TokenResult {
        let request = self.request(resource)?;
        let (response, request_time) = io.exchange(&request)?;
        cloud_shell_reply::cloud_shell_response(&response, &self.environment, request_time, io)
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

use std::collections::BTreeMap;

use secrecy::{ExposeSecret as _, SecretString};

use crate::azure_client_secret::token::AccessToken;
use crate::azure_client_secret::transport::Clock;

use super::cloud_shell_reply;
use super::decode::decode_hosted_identity_token;
use super::environment::HostedEnvironment;
use super::error::HostedIdentityError;
use super::msal_reply::{self, MsalKind};
use super::request::{
    APP_SERVICE_API_VERSION, AZURE_ML_API_VERSION, HostedRequest, HostedTransport,
    SERVICE_FABRIC_API_VERSION, cloud_shell_request, token_request,
};

const APP_SERVICE_PREFIX: &str =
    "App Service managed identity configuration not found in environment. ";
const AZURE_ML_PREFIX: &str = "Azure ML managed identity configuration not found in environment. ";
const SERVICE_FABRIC_PREFIX: &str =
    "Service Fabric managed identity configuration not found in environment. ";
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

    pub fn acquire(
        &self,
        kind: MsalKind,
        transport: &dyn HostedTransport,
        clock: &dyn Clock,
        resource: &str,
    ) -> Result<AccessToken, HostedIdentityError> {
        let request = self.request(kind, resource)?;
        let started = clock.now_unix_seconds();
        let response = transport
            .send(&request)
            .map_err(HostedIdentityError::Transport)?;
        let response_time = clock.now_unix_seconds();
        let reply = decode_hosted_identity_token(response.body.expose_secret().as_bytes())?
            .ok_or_else(|| HostedIdentityError::ReplyValue {
                field: "$".to_owned(),
            })?;
        let normalized = msal_reply::msal_reply(
            kind,
            &reply,
            &self.environment,
            response_time,
            response.body.expose_secret(),
        )?;
        let prefix = match kind {
            MsalKind::AppService => APP_SERVICE_PREFIX,
            MsalKind::AzureMl => AZURE_ML_PREFIX,
            MsalKind::ServiceFabric => SERVICE_FABRIC_PREFIX,
        };
        // the SDK reads its own clock after MSAL normalized the reply, so a slow request shifts expires_on
        msal_reply::complete_msal(normalized, prefix, started, clock.now_unix_seconds())
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

    pub fn acquire(
        &self,
        transport: &dyn HostedTransport,
        clock: &dyn Clock,
        resource: &str,
    ) -> Result<AccessToken, HostedIdentityError> {
        self.validate_identity()?;
        let endpoint = self
            .environment
            .msi_endpoint
            .as_deref()
            .filter(|endpoint| !endpoint.is_empty())
            .ok_or(HostedIdentityError::CloudShellUnavailable)?;
        let request = cloud_shell_request(
            endpoint,
            resource,
            &self.identity.identity_config,
            self.identity.user_client_id(),
        );
        let request_time = clock.now_unix_seconds();
        let response = transport
            .send(&request)
            .map_err(HostedIdentityError::Transport)?;
        let body = response.body.expose_secret();
        if body.is_empty() {
            return Err(HostedIdentityError::NoToken);
        }
        let reply = match decode_hosted_identity_token(body.as_bytes()) {
            Ok(reply) => reply,
            Err(HostedIdentityError::ReplyJson(source)) => {
                return Err(cloud_shell_reply::json_reply_failure(
                    response.content_type.as_deref(),
                    source,
                ));
            }
            Err(error) => return Err(error),
        };
        cloud_shell_reply::cloud_shell_token(reply.as_ref(), &self.environment, body, request_time)
    }

    fn validate_identity(&self) -> Result<(), HostedIdentityError> {
        if self.within_default_credential {
            return Ok(());
        }
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

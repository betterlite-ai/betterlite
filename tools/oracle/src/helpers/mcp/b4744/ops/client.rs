use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::tls::{TlsPolicy, Verification};
use betterlite_mcp::client::sessions::client::{SessionClient, SessionSettings};
use betterlite_mcp::client::sessions::negotiate::{Callbacks, ProtocolVersion};
use betterlite_mcp::client::sessions::transport::StdioConfig;
use betterlite_mcp::credentials::{
    AuthValue, CredentialError, McpAuthType, McpClient as CredentialClient,
    Request as CredentialRequest, ResolvedAuth,
};
use secrecy::SecretString;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::helpers::HelperError;

use super::super::fixtures::Fixture;
use super::super::input::{AuthKind, AuthValueInput, SettingsInput, TlsInput};
use super::invalid;

const fn auth_type(kind: AuthKind) -> McpAuthType {
    match kind {
        AuthKind::None => McpAuthType::None,
        AuthKind::ApiKey => McpAuthType::ApiKey,
        AuthKind::BearerToken => McpAuthType::BearerToken,
        AuthKind::Basic => McpAuthType::Basic,
        AuthKind::Authorization => McpAuthType::Authorization,
        AuthKind::Oauth2 => McpAuthType::Oauth2,
        AuthKind::Token => McpAuthType::Token,
        AuthKind::Oauth2TokenExchange => McpAuthType::Oauth2TokenExchange,
    }
}

fn auth_value(input: Option<&AuthValueInput>) -> Option<AuthValue> {
    let value = input?;
    match value {
        AuthValueInput::Text(text) => Some(AuthValue::Text(SecretString::from(text.clone()))),
        AuthValueInput::Headers(headers) => Some(AuthValue::Headers(
            headers
                .iter()
                .map(|(name, secret)| (name.clone(), SecretString::from(secret.clone())))
                .collect(),
        )),
    }
}

struct PresenceAuth {
    header_name: Option<String>,
}

impl ResolvedAuth for PresenceAuth {
    fn authenticate(
        &self,
        request: CredentialRequest,
    ) -> Result<CredentialRequest, CredentialError> {
        Ok(request)
    }

    fn close(&self) {}

    fn header_name(&self) -> Option<&str> {
        self.header_name.as_deref()
    }
}

pub(super) fn tls_policy(input: &TlsInput) -> TlsPolicy {
    let verification = match input.verification {
        super::super::input::TlsKind::Enabled => Verification::Enabled,
        super::super::input::TlsKind::Disabled => Verification::Disabled,
        super::super::input::TlsKind::CaFile => {
            Verification::CaFile(input.ca_file.clone().unwrap_or_default())
        }
    };
    TlsPolicy {
        verification,
        ca_file: input.ca_file.clone(),
        certificate: None,
        http2: false,
        ecdh_curve: None,
        user_agent: "betterlite-oracle".to_owned(),
    }
}

fn credentials(input: &SettingsInput) -> CredentialClient {
    let resolved = input.auth.resolved_auth.then(|| -> Box<dyn ResolvedAuth> {
        Box::new(PresenceAuth {
            header_name: input.auth.resolved_header_name.clone(),
        })
    });
    let aws = input
        .auth
        .aws_auth
        .then(|| -> Box<dyn ResolvedAuth> { Box::new(PresenceAuth { header_name: None }) });
    CredentialClient::new(
        input.server_url.clone(),
        auth_type(input.auth.auth_type),
        auth_value(input.auth.auth_value.as_ref()),
        input.auth.auth_header_name.clone(),
        input.auth.extra_headers.clone(),
        Duration::from_millis(input.timeout_ms),
        resolved,
        aws,
    )
}

fn session_settings(input: &SettingsInput) -> Result<SessionSettings, Box<HelperError>> {
    let protocol_version = ProtocolVersion::try_from(input.protocol_version.clone())
        .map_err(|error| invalid(error.to_string()))?;
    Ok(SessionSettings {
        transport_type: input.transport_type,
        stdio: input.stdio.as_ref().map(|stdio| StdioConfig {
            command: stdio.command.clone(),
            args: stdio.args.clone(),
            env: stdio.env.clone(),
        }),
        environment: input.environment.clone(),
        npm_cache_dir: input.npm_cache_dir.clone(),
        tls: tls_policy(&input.tls),
        protocol_version,
        client_version: input.client_version.clone(),
        callbacks: Callbacks {
            kinds: BTreeSet::new(),
        },
    })
}

pub(super) fn make_client(
    input: &SettingsInput,
    fixture: Fixture,
    tracker: TaskTracker,
    caller_cancelled: CancellationToken,
) -> Result<SessionClient<Fixture>, Box<HelperError>> {
    Ok(SessionClient {
        runtime: Arc::new(fixture),
        auth: credentials(input),
        settings: session_settings(input)?,
        tracker,
        caller_cancelled,
        last_initialize_instructions: None,
    })
}

use std::collections::BTreeMap;

use betterlite_azure::azure_hosted_identity::credential::{
    CloudShellCredential, HostedCredential, HostedIdentity,
};
use betterlite_azure::azure_hosted_identity::environment::HostedEnvironment;
use betterlite_azure::azure_hosted_identity::error::HostedIdentityError;
use betterlite_azure::azure_hosted_identity::msal_reply::MsalKind;
use betterlite_azure::azure_hosted_identity::protocol::select_hosted_protocol;
use betterlite_azure::azure_hosted_identity::request::{HostedIo, TokenResult};
use betterlite_config::loader::{ConfigError, ConfigLoader};
use betterlite_redact::error::RedactionError;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use fixture::{FixedClock, Recording, Reply, Selection};
use secrecy::ExposeSecret as _;
use serde::Deserialize;
use serde_json::json;

use crate::helpers::HelperError;

#[derive(Deserialize)]
pub(crate) struct Input {
    steps: Vec<Step>,
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub(crate) enum Step {
    Token(TokenStep),
    Select {
        #[serde(default)]
        environment: BTreeMap<String, String>,
        platform: Option<String>,
        #[serde(default)]
        files: BTreeMap<String, String>,
    },
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Protocol {
    AppService,
    AzureMl,
    ServiceFabric,
    CloudShell,
}

#[derive(Deserialize)]
pub(crate) struct TokenStep {
    protocol: Protocol,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    client_id: Option<String>,
    #[serde(default)]
    identity_config: BTreeMap<String, String>,
    #[serde(default)]
    within_default_credential: bool,
    resource: String,
    now: i64,
    replies: Vec<Reply>,
}

fn category(error: &HostedIdentityError) -> String {
    match error {
        HostedIdentityError::TokenRequest { .. }
        | HostedIdentityError::CloudShellUnavailable
        | HostedIdentityError::UnsupportedContentType { .. } => "unavailable".to_owned(),
        HostedIdentityError::ServiceFabricIdentityConfig
        | HostedIdentityError::NoToken
        | HostedIdentityError::UnexpectedResponse { .. }
        | HostedIdentityError::JsonContentType(_) => "client_authentication".to_owned(),
        HostedIdentityError::CloudShellClientId
        | HostedIdentityError::CloudShellIdentityConfig
        | HostedIdentityError::ReplyValue { .. }
        | HostedIdentityError::ExpiryOverflow => "invalid_value".to_owned(),
        HostedIdentityError::ReplyJson(_) => "invalid_json".to_owned(),
        HostedIdentityError::MissingField { field } => format!("missing_{field}"),
        HostedIdentityError::MissingEnvironment { .. } | HostedIdentityError::Transport(_) => {
            "transport".to_owned()
        }
    }
}

fn acquire(step: &TokenStep, io: &HostedIo<'_>) -> TokenResult {
    let environment = HostedEnvironment::from_environment(&step.environment);
    let identity = HostedIdentity {
        client_id: step.client_id.clone(),
        identity_config: step.identity_config.clone(),
    };
    let hosted = HostedCredential {
        environment: environment.clone(),
        identity: identity.clone(),
    };
    match step.protocol {
        Protocol::AppService => hosted.acquire(MsalKind::AppService, io, &step.resource),
        Protocol::AzureMl => hosted.acquire(MsalKind::AzureMl, io, &step.resource),
        Protocol::ServiceFabric => hosted.acquire(MsalKind::ServiceFabric, io, &step.resource),
        Protocol::CloudShell => CloudShellCredential {
            environment,
            identity,
            within_default_credential: step.within_default_credential,
        }
        .acquire(io, &step.resource),
    }
}

#[derive(Debug, thiserror::Error)]
enum Failure {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Redaction(#[from] RedactionError),
}

impl From<Failure> for Box<HelperError> {
    fn from(failure: Failure) -> Self {
        let class = match &failure {
            Failure::Config(_) => "ConfigError",
            Failure::Redaction(_) => "Redaction",
        };
        Self::new(HelperError {
            class: class.to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: failure.to_string(),
        })
    }
}

fn token_step(step: &TokenStep) -> Result<serde_json::Value, Failure> {
    let transport = Recording::new(&step.replies);
    let clock = FixedClock { now: step.now };
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)?;
    let io = HostedIo {
        transport: &transport,
        clock: &clock,
        redactor: &redactor,
    };
    let outcome = match acquire(step, &io) {
        Ok(token) => json!({
            "kind": "token",
            "token": token.token.expose_secret(),
            "expires_on": token.expires_on,
            "token_type": token.token_type,
            "refresh_on": token.refresh_on,
            "bearer": token.token.expose_secret(),
        }),
        Err(error) => json!({"kind": "error", "category": category(&error)}),
    };
    let requests = transport
        .requests
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    Ok(json!({"requests": requests, "outcome": outcome}))
}

fn select_step(
    environment: &BTreeMap<String, String>,
    platform: Option<&str>,
    files: &BTreeMap<String, String>,
) -> Result<serde_json::Value, Failure> {
    let settings = ConfigLoader {
        lookup: environment.clone(),
    }
    .load("")?
    .sections
    .azure_arc_identity;
    let host = Selection {
        environment,
        platform,
        files,
        settings,
    };
    let selected = select_hosted_protocol(&HostedEnvironment::from_environment(environment), &host);
    Ok(json!({"selected": selected.name()}))
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let steps = input
            .steps
            .iter()
            .map(|step| match step {
                Step::Token(token) => token_step(token),
                Step::Select {
                    environment,
                    platform,
                    files,
                } => select_step(environment, platform.as_deref(), files),
            })
            .collect::<Result<Vec<_>, Failure>>()?;
        Ok(json!({"steps": steps}))
    }
}

mod fixture;
mod tests;

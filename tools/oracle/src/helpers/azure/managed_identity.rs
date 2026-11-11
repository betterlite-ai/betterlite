use std::collections::BTreeMap;

use betterlite_azure::azure_arc_identity::{ArcHost, ArcPlatform};
use betterlite_azure::azure_hosted_identity::protocol::HostedProtocol;
use betterlite_azure::azure_managed_identity::ManagedIdentityToken;
use betterlite_azure::azure_managed_identity::error::ManagedIdentityError as Identity;
use betterlite_azure::azure_managed_identity::imds::ProbePolicy;
use betterlite_azure::azure_managed_identity::selection::ManagedIdentityKind;
use betterlite_azure::oidc_exchange::OidcExchangeError as Oidc;
use secrecy::ExposeSecret as _;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::HelperError;

use self::fixture::Fixture;
use self::fixture::transport::{Reply, Shared};

mod fixture;
mod tests;

const DEFAULT_SCOPE: &str = "https://cognitiveservices.azure.com/.default";
const NOW: i64 = 1_700_000_000;

#[derive(Deserialize)]
pub(crate) struct Input {
    steps: Vec<Step>,
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub(crate) enum Step {
    Select(SelectStep),
    Token(TokenStep),
}

#[derive(Deserialize)]
pub(crate) struct SelectStep {
    #[serde(default)]
    environment: BTreeMap<String, String>,
    client_id: Option<String>,
    #[serde(default)]
    exclude_workload_identity: bool,
    #[serde(default)]
    files: BTreeMap<String, String>,
}

#[derive(Deserialize)]
pub(crate) struct TokenStep {
    #[serde(default)]
    environment: BTreeMap<String, String>,
    client_id: Option<String>,
    scope: Option<String>,
    now: Option<i64>,
    probe: Option<bool>,
    #[serde(default)]
    within_credential_chain: bool,
    #[serde(default)]
    exclude_workload_identity: bool,
    #[serde(default)]
    files: BTreeMap<String, String>,
    #[serde(default)]
    replies: Vec<Reply>,
}

impl ArcHost for Fixture {
    fn platform(&self) -> ArcPlatform {
        ArcPlatform::Linux
    }

    fn environment(&self, name: &str) -> Option<String> {
        self.environment.get(name).cloned()
    }

    fn program_data(&self) -> String {
        "%ProgramData%".to_owned()
    }

    fn program_files(&self) -> String {
        "${ProgramFiles}".to_owned()
    }

    fn file_size(&self, path: &str) -> std::io::Result<u64> {
        u64::try_from(self.read_text(path)?.len()).map_err(std::io::Error::other)
    }

    fn read_text(&self, path: &str) -> std::io::Result<String> {
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, path.to_owned()))
    }

    fn now_unix_seconds(&self) -> i64 {
        self.now
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum Failure {
    #[error(transparent)]
    Fixture(#[from] fixture::FixtureError),
}

impl From<Failure> for Box<HelperError> {
    fn from(failure: Failure) -> Self {
        Self::new(HelperError {
            class: "FixtureError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: failure.to_string(),
        })
    }
}

// hosted and Arc protocol failures are other tasks' oracle cases; the fixture transport refuses them
const fn category(error: &Identity) -> &'static str {
    match error {
        Identity::ImdsUnavailable
        | Identity::TokenRequest { .. }
        | Identity::ImdsError { .. }
        | Identity::NoEndpoint
        | Identity::NoIdentityAssigned
        | Identity::AuthenticationUnavailable
        | Identity::Oidc(Oidc::UnresolvedOidcToken) => "unavailable",
        Identity::ClientAuthentication { .. }
        | Identity::Oidc(
            Oidc::MissingClientTenant
            | Oidc::AccessTokenMissing
            | Oidc::ExpiresInMissing
            | Oidc::TokenStatus { .. },
        ) => "client_authentication",
        Identity::ScopeCount
        | Identity::WorkloadClientId
        | Identity::ImdsReply(_)
        | Identity::ExpiryOverflow => "invalid_value",
        Identity::Oidc(Oidc::TokenJson { .. } | Oidc::TokenFieldType { .. }) => "invalid_json",
        Identity::TokenFile { .. }
        | Identity::TokenFileEncoding { .. }
        | Identity::ArcEndpoint
        | Identity::Hosted(_)
        | Identity::Arc(_)
        | Identity::Oidc(Oidc::TokenTransport { .. } | Oidc::Cache { .. } | Oidc::CachePoisoned) => {
            "transport"
        }
    }
}

fn token_outcome(selection: ManagedIdentityKind, token: &ManagedIdentityToken) -> Value {
    if selection == ManagedIdentityKind::Workload {
        return json!({"kind": "token", "token": token.token.expose_secret()});
    }
    json!({
        "kind": "token",
        "token": token.token.expose_secret(),
        "expires_on": token.expires_on,
        "token_type": token.token_type,
        "refresh_on": token.refresh_on,
    })
}

fn select_step(step: &SelectStep) -> Result<Value, Failure> {
    let fixture = Fixture::new(&step.environment, &step.files, NOW, Shared::new(Vec::new()))?;
    let credential = fixture.credential(
        step.client_id.as_deref(),
        step.exclude_workload_identity,
        ProbePolicy::default(),
    );
    Ok(match credential.select() {
        Ok(selection) => json!({
            "kind": "selection",
            "selected": selection.kind.name(),
            "inner": credential.inner_protocol(&selection).map(HostedProtocol::name),
        }),
        Err(error) => json!({"kind": "error", "category": category(&error)}),
    })
}

fn token_step(step: &TokenStep) -> Result<Value, Failure> {
    let fixture = Fixture::new(
        &step.environment,
        &step.files,
        step.now.unwrap_or(NOW),
        Shared::new(step.replies.clone()),
    )?;
    let credential = fixture.credential(
        step.client_id.as_deref(),
        step.exclude_workload_identity,
        ProbePolicy {
            enable: step.probe,
            within_credential_chain: step.within_credential_chain,
        },
    );
    let io = fixture.io();
    let scope = step.scope.as_deref().unwrap_or(DEFAULT_SCOPE);
    let (mut outcome, bearer) = match credential.select() {
        Err(error) => (
            json!({"kind": "error", "category": category(&error)}),
            Value::Null,
        ),
        Ok(selection) => match credential.token(&io, &[scope]) {
            Ok(token) => (
                token_outcome(selection.kind, &token),
                json!(token.token.expose_secret()),
            ),
            Err(error) => (
                json!({"kind": "error", "category": category(&error)}),
                Value::Null,
            ),
        },
    };
    if let Some(map) = outcome.as_object_mut() {
        map.insert("bearer".to_owned(), bearer);
        map.insert("requests".to_owned(), json!(fixture.shared.requests()));
        map.insert("sleeps".to_owned(), json!(fixture.shared.sleeps()));
    }
    Ok(outcome)
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let steps = input
            .steps
            .iter()
            .map(|step| match step {
                Step::Select(select) => select_step(select),
                Step::Token(token) => token_step(token),
            })
            .collect::<Result<Vec<_>, Failure>>()?;
        Ok(json!({"steps": steps}))
    }
}

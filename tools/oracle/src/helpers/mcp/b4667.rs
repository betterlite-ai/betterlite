mod fakes;
mod inspect;
mod recorders;
mod resolve;
mod tests;

use secrecy::ExposeSecret;
use serde::Deserialize;
use serde_json::{Value, json};

use betterlite_mcp::outbound_credentials::adapter;
use betterlite_mcp::outbound_credentials::adapter::UserPrincipal;
use betterlite_mcp::server_record::MCPServer;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    ToSubject {
        principal: Option<Principal>,
        subject_token: Option<String>,
    },
    ToServerSpec {
        server: MCPServer,
    },
    ProtectedResourcePath {
        server: MCPServer,
        root_path: String,
        scalar_root_path: String,
    },
    UserOauthChallenge {
        server: MCPServer,
        root_path: String,
        scalar_root_path: String,
    },
    TokenExchangeChallenge {
        server: MCPServer,
        root_path: String,
        scalar_root_path: String,
        claims: Option<String>,
    },
    CredentialResolver {
        server: MCPServer,
        subject: Box<resolve::SubjectInput>,
        #[serde(default)]
        collaborators: Box<fakes::CollaboratorsInput>,
        actions: Vec<resolve::Action>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Principal {
    #[serde(rename = "org_id")]
    org: Option<String>,
    #[serde(rename = "team_id")]
    team: Option<String>,
    #[serde(rename = "user_id")]
    user: Option<String>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ToSubject {
                principal,
                subject_token,
            } => Ok(subject_answer(principal.as_ref(), subject_token.as_deref())),
            Input::ToServerSpec { server } => inspect::spec_answer(server),
            Input::ProtectedResourcePath {
                server,
                root_path,
                scalar_root_path,
            } => Ok(inspect::resource_path(server, root_path, scalar_root_path)),
            Input::UserOauthChallenge {
                server,
                root_path,
                scalar_root_path,
            } => Ok(inspect::user_challenge(server, root_path, scalar_root_path)),
            Input::TokenExchangeChallenge {
                server,
                root_path,
                scalar_root_path,
                claims,
            } => Ok(inspect::exchange_challenge(
                server,
                root_path,
                scalar_root_path,
                claims.as_deref(),
            )),
            Input::CredentialResolver {
                server,
                subject,
                collaborators,
                actions,
            } => resolve::answer(server, subject, collaborators, actions),
        }
    }
}

fn subject_answer(principal: Option<&Principal>, subject_token: Option<&str>) -> Value {
    let user = principal.map(|fields| UserPrincipal {
        org_id: fields.org.clone(),
        team_id: fields.team.clone(),
        user_id: fields.user.clone(),
    });
    let subject = adapter::to_subject(user.as_ref(), subject_token);
    json!({
        "tenant_id": subject.tenant_id,
        "subject_id": subject.subject_id,
        "inbound_token": subject.inbound_token.as_ref().map(ExposeSecret::expose_secret),
    })
}

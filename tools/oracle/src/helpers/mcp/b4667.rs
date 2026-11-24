mod fakes;
mod inspect;
mod recorders;
mod resolve;
mod tests;

use secrecy::ExposeSecret as _;
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
        subject: resolve::SubjectInput,
        #[serde(default)]
        collaborators: fakes::CollaboratorsInput,
        actions: Vec<resolve::Action>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Principal {
    org_id: Option<String>,
    team_id: Option<String>,
    user_id: Option<String>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ToSubject {
                principal,
                subject_token,
            } => Ok(to_subject_answer(
                principal.as_ref(),
                subject_token.as_deref(),
            )),
            Input::ToServerSpec { server } => inspect::spec_answer(server),
            Input::ProtectedResourcePath {
                server,
                root_path,
                scalar_root_path,
            } => Ok(Value::String(
                adapter::challenge::oauth_protected_resource_path(
                    root_path,
                    scalar_root_path,
                    server,
                ),
            )),
            Input::UserOauthChallenge {
                server,
                root_path,
                scalar_root_path,
            } => Ok(inspect::challenge_answer(
                &adapter::challenge::raise_user_oauth_challenge(
                    server,
                    root_path,
                    scalar_root_path,
                ),
            )),
            Input::TokenExchangeChallenge {
                server,
                root_path,
                scalar_root_path,
                claims,
            } => Ok(inspect::challenge_answer(
                &adapter::challenge::raise_token_exchange_challenge(
                    server,
                    root_path,
                    scalar_root_path,
                    claims.as_deref(),
                ),
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

fn to_subject_answer(principal: Option<&Principal>, subject_token: Option<&str>) -> Value {
    let user = principal.map(|fields| UserPrincipal {
        org_id: fields.org_id.clone(),
        team_id: fields.team_id.clone(),
        user_id: fields.user_id.clone(),
    });
    let subject = adapter::to_subject(user.as_ref(), subject_token);
    json!({
        "tenant_id": subject.tenant_id,
        "subject_id": subject.subject_id,
        "inbound_token": subject.inbound_token.as_ref().map(|token| token.expose_secret()),
    })
}

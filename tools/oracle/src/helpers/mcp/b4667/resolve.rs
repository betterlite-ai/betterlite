use std::collections::BTreeMap;

use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;

use betterlite_mcp::credential_contracts::errors::CredError;
use betterlite_mcp::credential_contracts::server_spec::{ServerSpec, Subject};
use betterlite_mcp::credentials::{Request, ResolvedAuth};
use betterlite_mcp::outbound_credentials::adapter;
use betterlite_mcp::outbound_credentials::resolver::UpstreamCredentialProvider;
use betterlite_mcp::outbound_credentials::resolver::source::resolve_with_source;
use betterlite_mcp::server_record::MCPServer;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::HelperResult;

use super::fakes::{self, CollaboratorsInput};

const UPSTREAM_URL: &str = "https://upstream.example.com/mcp";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SubjectInput {
    tenant_id: String,
    subject_id: String,
    inbound_token: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Action {
    Resolve,
    ResolveWithSource,
    HasUserToken,
    Invalidate,
}

pub(super) fn answer(
    server: &MCPServer,
    subject: &SubjectInput,
    collaborators: &CollaboratorsInput,
    actions: &[Action],
) -> HelperResult {
    let Some(spec) = adapter::to_server_spec(server).or_value_error()? else {
        return Ok(Value::Null);
    };
    let fixtures = fakes::build(collaborators);
    let provider = UpstreamCredentialProvider {
        collaborators: fixtures.collaborators,
    };
    let caller = subject.to_subject();
    let runtime = Builder::new_current_thread()
        .enable_time()
        .build()
        .or_value_error()?;
    let records = runtime.block_on(run_actions(&provider, &caller, &spec, actions));
    Ok(json!({
        "actions": records,
        "token_endpoint_calls": fixtures.endpoint.calls(),
        "exchanger_exchanges": fixtures.exchanger.exchanges(),
        "exchanger_invalidations": fixtures.exchanger.invalidations(),
    }))
}

impl SubjectInput {
    fn to_subject(&self) -> Subject {
        Subject {
            tenant_id: self.tenant_id.clone(),
            subject_id: self.subject_id.clone(),
            inbound_token: self
                .inbound_token
                .as_ref()
                .map(|token| SecretString::from(token.as_str())),
        }
    }
}

async fn run_actions(
    provider: &UpstreamCredentialProvider,
    subject: &Subject,
    server: &ServerSpec,
    actions: &[Action],
) -> Vec<Value> {
    let mut records = Vec::with_capacity(actions.len());
    for action in actions {
        let record = match action {
            Action::Resolve => resolve_answer(provider, subject, server, AnswerMode::Plain).await,
            Action::ResolveWithSource => {
                resolve_answer(provider, subject, server, AnswerMode::WithSource).await
            }
            Action::HasUserToken => json!({
                "action": "has_user_token",
                "ok": true,
                "has_user_token": provider.has_user_token(subject, server).await,
            }),
            Action::Invalidate => {
                provider.invalidate_credentials(subject, server).await;
                json!({"action": "invalidate", "ok": true})
            }
        };
        records.push(record);
    }
    records
}

#[derive(Clone, Copy)]
enum AnswerMode {
    Plain,
    WithSource,
}

impl AnswerMode {
    const fn label(self) -> &'static str {
        match self {
            Self::Plain => "resolve",
            Self::WithSource => "resolve_with_source",
        }
    }
}

async fn resolve_answer(
    provider: &UpstreamCredentialProvider,
    subject: &Subject,
    server: &ServerSpec,
    mode: AnswerMode,
) -> Value {
    let action = mode.label();
    match resolve_with_source(provider, subject, server).await {
        Ok(resolved) => {
            let headers = applied(resolved.auth.as_ref());
            let source = resolved.source.to_string();
            match mode {
                AnswerMode::Plain => json!({"action": action, "ok": true, "headers": headers}),
                AnswerMode::WithSource => json!({
                    "action": action,
                    "ok": true,
                    "headers": headers,
                    "source": source,
                }),
            }
        }
        Err(error) => error_answer(action, &error),
    }
}

fn applied(auth: &dyn ResolvedAuth) -> BTreeMap<String, String> {
    let request = Request {
        method: "POST".to_owned(),
        url: UPSTREAM_URL.to_owned(),
        headers: BTreeMap::new(),
    };
    match auth.authenticate(request) {
        Ok(authenticated) => authenticated
            .headers
            .into_iter()
            .map(|(name, value)| (name.to_lowercase(), value))
            .collect(),
        Err(_error) => BTreeMap::from([("authorization".to_owned(), "apply failed".to_owned())]),
    }
}

fn error_answer(action: &str, error: &CredError) -> Value {
    json!({
        "action": action,
        "ok": false,
        "tag": error_tag(error),
        "summary": error.to_string(),
        "www_authenticate": www_authenticate(error),
    })
}

const fn error_tag(error: &CredError) -> &'static str {
    match error {
        CredError::Unauthorized(_unauthorized) => "unauthorized",
        CredError::Misconfigured(_detail) => "misconfigured",
        CredError::UrlCredentialsNotAllowed => "url_credentials_not_allowed",
        CredError::UpstreamUnavailable(_detail) => "upstream_unavailable",
        CredError::UnsupportedMode(_detail) => "unsupported_mode",
        CredError::PreconditionRequired(_detail) => "precondition_required",
        CredError::NotImplemented(_detail) => "not_implemented",
    }
}

fn www_authenticate(error: &CredError) -> Option<&str> {
    let CredError::Unauthorized(unauthorized) = error else {
        return None;
    };
    unauthorized.www_authenticate.as_deref()
}

use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::sync::Mutex;

use betterlite_azure::azure_arc_identity::error::ArcIdentityError;
use betterlite_azure::azure_arc_identity::{
    ArcHost, ArcIdentity, ArcPlatform, ArcRequest, ArcResponse, ArcTransport, acquire_arc_token,
    select_arc_endpoint,
};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
pub(crate) struct Fixture {
    platform: String,
    environment: BTreeMap<String, String>,
    files: BTreeMap<String, String>,
    now: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Identity {
    System,
    User,
}

#[derive(Clone, Deserialize)]
pub(crate) struct Reply {
    status: u16,
    headers: BTreeMap<String, String>,
    body: String,
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub(crate) enum Input {
    Endpoint(Fixture),
    Token {
        #[serde(flatten)]
        fixture: Fixture,
        resource: String,
        identity: Identity,
        responses: Vec<Reply>,
    },
}

impl ArcHost for Fixture {
    fn platform(&self) -> ArcPlatform {
        match self.platform.as_str() {
            "linux" => ArcPlatform::Linux,
            "win32" => ArcPlatform::Windows,
            other => ArcPlatform::Other(other.to_owned()),
        }
    }

    fn environment(&self, name: &str) -> Option<String> {
        self.environment.get(name).cloned()
    }

    fn file_size(&self, path: &str) -> io::Result<u64> {
        let text = self.read_text(path)?;
        u64::try_from(text.len()).map_err(io::Error::other)
    }

    fn read_text(&self, path: &str) -> io::Result<String> {
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, path.to_owned()))
    }

    fn now_unix_seconds(&self) -> i64 {
        self.now
    }
}

struct Recording {
    replies: Mutex<VecDeque<Reply>>,
    requests: Mutex<Vec<serde_json::Value>>,
}

impl ArcTransport for Recording {
    fn get(&self, request: &ArcRequest<'_>) -> io::Result<ArcResponse> {
        let mut headers = BTreeMap::from([("Metadata", "true".to_owned())]);
        if let Some(secret) = request.key_secret {
            headers.insert("Authorization", format!("Basic {}", secret.expose_secret()));
        }
        self.requests
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?
            .push(json!({
                "url": request.endpoint,
                "params": {"api-version": "2020-06-01", "resource": request.resource},
                "headers": headers,
            }));
        let reply = self
            .replies
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?
            .pop_front()
            .ok_or_else(|| io::Error::other("no reply queued"))?;
        Ok(ArcResponse {
            status: reply.status,
            headers: reply.headers.into_iter().collect(),
            body: SecretString::from(reply.body),
        })
    }
}

fn error_class(error: &ArcIdentityError) -> &'static str {
    match error {
        ArcIdentityError::TokenRequest { .. } => "CredentialUnavailableError",
        ArcIdentityError::KeyFile(source) if source.kind() == io::ErrorKind::NotFound => {
            "FileNotFoundError"
        }
        ArcIdentityError::KeyFile(_) | ArcIdentityError::Transport(_) => "OSError",
        ArcIdentityError::ReplyValue { .. } => "ValueError",
        ArcIdentityError::UnrecognizableChallenge { .. }
        | ArcIdentityError::UnsupportedPlatform { .. }
        | ArcIdentityError::KeyFileTooLarge
        | ArcIdentityError::UserAssignedIdentity => "ClientAuthenticationError",
    }
}

fn acquire(
    fixture: &Fixture,
    resource: &str,
    identity: &Identity,
    responses: &[Reply],
) -> serde_json::Value {
    let Some(endpoint) = select_arc_endpoint(fixture) else {
        return json!({"requests": [], "result": {"kind": "no_endpoint"}, "bearer": null});
    };
    let transport = Recording {
        replies: Mutex::new(responses.iter().cloned().collect()),
        requests: Mutex::new(Vec::new()),
    };
    let requested = match identity {
        Identity::System => ArcIdentity::SystemAssigned,
        Identity::User => ArcIdentity::UserAssigned,
    };
    let outcome = acquire_arc_token(fixture, &transport, &endpoint, resource, requested);
    let requests = transport
        .requests
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (result, bearer) = match outcome {
        Ok(token) => (
            json!({
                "kind": "token",
                "token": token.token.expose_secret(),
                "expires_on": token.expires_on,
                "token_type": token.token_type,
                "refresh_on": token.refresh_on,
            }),
            json!(token.token.expose_secret()),
        ),
        Err(error) => (
            json!({"kind": "error", "class": error_class(&error), "message": error.to_string()}),
            json!(null),
        ),
    };
    json!({"requests": requests, "result": result, "bearer": bearer})
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        Ok(match input {
            Input::Endpoint(fixture) => json!({"endpoint": select_arc_endpoint(fixture)}),
            Input::Token {
                fixture,
                resource,
                identity,
                responses,
            } => acquire(fixture, resource, identity, responses),
        })
    }
}

mod tests;

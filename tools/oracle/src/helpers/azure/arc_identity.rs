use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::sync::Mutex;

use betterlite_azure::azure_arc_identity::error::ArcIdentityError;
use betterlite_azure::azure_arc_identity::{
    ArcHost, ArcIdentity, ArcPlatform, ArcRequest, ArcResponse, ArcTransport, acquire_arc_token,
    select_arc_endpoint,
};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::azure_arc_identity::Settings;
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

struct Host<'fixture> {
    fixture: &'fixture Fixture,
    settings: Settings,
}

impl<'fixture> Host<'fixture> {
    fn load(fixture: &'fixture Fixture) -> Result<Self, Box<crate::helpers::HelperError>> {
        let config = ConfigLoader {
            lookup: fixture.environment.clone(),
        }
        .load("")?;
        Ok(Self {
            fixture,
            settings: config.sections.azure_arc_identity,
        })
    }
}

impl ArcHost for Host<'_> {
    fn platform(&self) -> ArcPlatform {
        match self.fixture.platform.as_str() {
            "linux" => ArcPlatform::Linux,
            "win32" => ArcPlatform::Windows,
            other => ArcPlatform::Other(other.to_owned()),
        }
    }

    fn environment(&self, name: &str) -> Option<String> {
        self.fixture.environment.get(name).cloned()
    }

    fn program_data(&self) -> String {
        self.settings.program_data.clone()
    }

    fn program_files(&self) -> String {
        self.settings.program_files.clone()
    }

    fn file_size(&self, path: &str) -> io::Result<u64> {
        let text = self.read_text(path)?;
        u64::try_from(text.len()).map_err(io::Error::other)
    }

    fn read_text(&self, path: &str) -> io::Result<String> {
        self.fixture
            .files
            .get(path)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, path.to_owned()))
    }

    fn now_unix_seconds(&self) -> i64 {
        self.fixture.now
    }
}

struct Recording {
    replies: Mutex<VecDeque<Reply>>,
    requests: Mutex<Vec<serde_json::Value>>,
}

impl ArcTransport for Recording {
    fn get(&self, request: &ArcRequest) -> io::Result<ArcResponse> {
        let params: BTreeMap<_, _> = request.query.iter().cloned().collect();
        let headers: BTreeMap<_, _> = request
            .headers
            .iter()
            .map(|(name, value)| (*name, value.expose_secret()))
            .collect();
        self.requests
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?
            .push(json!({"url": request.url, "params": params, "headers": headers}));
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
    host: &Host<'_>,
    resource: &str,
    identity: &Identity,
    responses: &[Reply],
) -> serde_json::Value {
    let Some(endpoint) = select_arc_endpoint(host) else {
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
    let outcome = acquire_arc_token(host, &transport, &endpoint, resource, requested);
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
            Input::Endpoint(fixture) => {
                json!({"endpoint": select_arc_endpoint(&Host::load(fixture)?)})
            }
            Input::Token {
                fixture,
                resource,
                identity,
                responses,
            } => acquire(&Host::load(fixture)?, resource, identity, responses),
        })
    }
}

mod tests;

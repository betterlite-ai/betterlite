use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::io;
use std::path::Path;
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_openai::copilot::api_key::clock::Clock;
use betterlite_openai::copilot::api_key::transport::{ApiKeyTransport, Get};
use betterlite_openai::copilot::device_auth::Authenticator;
use betterlite_openai::copilot::device_auth::error::DeviceAuthError;
use betterlite_openai::copilot::device_auth::files::{FileAccess, LocalFiles};
use betterlite_openai::copilot::device_auth::output::OperatorOutput;
use betterlite_openai::copilot::device_auth::sleeper::Sleeper;
use betterlite_openai::copilot::device_auth::transport::{
    Post, Reply, Transport, TransportFailure,
};
use betterlite_openai::openai_base_config::Headers;
use secrecy::ExposeSecret as _;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

mod tests;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScriptedReply {
    status: u16,
    body: Value,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Action {
    GetApiKey,
    GetApiBase,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    action: Action,
    now: i64,
    api_key_file: Option<String>,
    access_token_file: Option<String>,
    #[serde(default)]
    api_replies: Vec<ScriptedReply>,
    #[serde(default)]
    fail_write: bool,
}

#[derive(Debug, thiserror::Error)]
enum FixtureError {
    #[error("fixture IO failed")]
    Io(#[from] io::Error),
    #[error("fixture config failed")]
    Config(#[from] betterlite_config::loader::ConfigError),
    #[error("fixture authenticator failed")]
    Auth(#[from] DeviceAuthError),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> HelperResult {
        run_fixture(input).map_err(|error| {
            Box::new(HelperError {
                class: "CopilotKeyFixture".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
    }
}

struct Files {
    fail_write: bool,
}

impl FileAccess for Files {
    fn ensure_dir(&mut self, path: &Path) -> io::Result<()> {
        LocalFiles.ensure_dir(path)
    }

    fn read(&mut self, path: &Path) -> impl Future<Output = io::Result<Vec<u8>>> + Send {
        let mut files = LocalFiles;
        async move { files.read(path).await }
    }

    fn write(
        &mut self,
        path: &Path,
        content: &[u8],
    ) -> impl Future<Output = io::Result<()>> + Send {
        let destination = if self.fail_write {
            path.join("persist")
        } else {
            path.to_path_buf()
        };
        async move {
            let mut files = LocalFiles;
            files.write(&destination, content).await
        }
    }
}

struct Scripted {
    replies: VecDeque<ScriptedReply>,
    requests: Vec<Value>,
}

fn marker(token: &str) -> String {
    if token.is_empty() {
        String::new()
    } else {
        String::from("<api-key>")
    }
}

fn normalize_authorization(value: &str) -> String {
    match value.split_once(' ') {
        Some((scheme, _token)) => format!("{scheme} <access-token>"),
        None => String::from("<access-token>"),
    }
}

fn recorded_headers(headers: &Headers) -> BTreeMap<String, Value> {
    headers
        .iter()
        .map(|(name, value)| {
            let normalized = if name == "authorization" {
                normalize_authorization(value)
            } else {
                value.clone()
            };
            (name.clone(), Value::String(normalized))
        })
        .collect()
}

fn record(method: &str, url: &str, headers: &Headers, body: &Value) -> Value {
    json!({
        "method": method,
        "url": url,
        "headers": recorded_headers(headers),
        "body": body,
    })
}

impl ApiKeyTransport for Scripted {
    fn get<'call>(
        &'call mut self,
        get: Get<'call>,
    ) -> impl Future<Output = Result<Reply, TransportFailure>> + Send + 'call {
        self.requests
            .push(record("GET", get.url, &get.headers, &Value::Null));
        ready(match self.replies.pop_front() {
            Some(scripted) => Ok(Reply {
                status: scripted.status,
                body: serde_json::to_vec(&scripted.body).unwrap_or_default(),
            }),
            None => Err(TransportFailure::Unavailable),
        })
    }
}

impl Transport for Scripted {
    fn post_json<'call>(
        &'call mut self,
        post: Post<'call>,
    ) -> impl Future<Output = Result<Reply, TransportFailure>> + Send + 'call {
        let body = serde_json::from_slice::<Value>(&post.body).unwrap_or(Value::Null);
        self.requests
            .push(record("POST", post.url, &post.headers, &body));
        ready(Err(TransportFailure::Unavailable))
    }
}

struct ScriptClock {
    now: i64,
}

impl Clock for ScriptClock {
    fn now(&self) -> i64 {
        self.now
    }
}

impl Sleeper for ScriptClock {
    fn sleep(&mut self, _duration: Duration) -> impl Future<Output = ()> + Send {
        ready(())
    }
}

#[derive(Debug, Default)]
struct NoOutput;

impl OperatorOutput for NoOutput {
    fn instructions(&mut self, _message: &str) {}
}

type Fixture = Authenticator<Files, ScriptClock, Scripted, NoOutput>;

fn fixture(input: &Input, directory: &Path) -> Result<Fixture, FixtureError> {
    let mut settings = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("")?
    .sections
    .copilot_device;
    settings.token_dir = directory.display().to_string();
    if input.fail_write {
        settings.api_key_file = String::from("persist/api-key.json");
    }
    if let Some(content) = &input.access_token_file {
        std::fs::write(directory.join(&settings.access_token_file), content)?;
    }
    if let Some(content) = &input.api_key_file {
        std::fs::write(directory.join("api-key.json"), content)?;
    }
    let transport = Scripted {
        replies: input.api_replies.iter().cloned().collect(),
        requests: Vec::new(),
    };
    Ok(Authenticator::new(
        settings,
        Files {
            fail_write: input.fail_write,
        },
        ScriptClock { now: input.now },
        transport,
        NoOutput,
    )?)
}

fn normalize_record(record: &mut Value) {
    let Value::Object(fields) = record else {
        return;
    };
    let normalized = match fields.get("token") {
        Some(Value::String(token)) => marker(token),
        _ => return,
    };
    fields.insert(String::from("token"), Value::String(normalized));
}

fn read_record(directory: &Path) -> Value {
    let Ok(bytes) = std::fs::read(directory.join("api-key.json")) else {
        return Value::Null;
    };
    let mut record = serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null);
    normalize_record(&mut record);
    record
}

fn run_fixture(input: &Input) -> Result<Value, FixtureError> {
    let directory = tempfile::tempdir()?;
    let mut authenticator = fixture(input, directory.path())?;
    let runtime = tokio::runtime::Builder::new_current_thread().build()?;
    let (result, error_status) = match input.action {
        Action::GetApiKey => match runtime.block_on(authenticator.get_api_key()) {
            Ok(token) => (
                token.as_ref().map_or(Value::Null, |cached| {
                    Value::String(marker(cached.expose_secret()))
                }),
                Value::Null,
            ),
            Err(error) => (Value::Null, json!(error.status())),
        },
        Action::GetApiBase => (
            runtime
                .block_on(authenticator.get_api_base())
                .map_or(Value::Null, Value::String),
            Value::Null,
        ),
    };
    Ok(json!({
        "result": result,
        "error_status": error_status,
        "file": read_record(directory.path()),
        "requests": authenticator.transport.requests,
    }))
}

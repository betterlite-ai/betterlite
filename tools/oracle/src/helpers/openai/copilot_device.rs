use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::io;
use std::path::Path;
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_openai::copilot::device_auth::files::{FileAccess, LocalFiles};
use betterlite_openai::copilot::device_auth::output::OperatorOutput;
use betterlite_openai::copilot::device_auth::sleeper::Sleeper;
use betterlite_openai::copilot::device_auth::transport::{
    Post, Reply, Transport, TransportFailure,
};
use betterlite_openai::copilot::device_auth::{Authenticator, DeviceAuthError};
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    access_token_file: Option<String>,
    #[serde(default)]
    device_replies: Vec<ScriptedReply>,
    #[serde(default)]
    access_replies: Vec<ScriptedReply>,
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
                class: "CopilotDeviceFixture".to_owned(),
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

    fn read(&mut self, path: &Path) -> io::Result<Vec<u8>> {
        LocalFiles.read(path)
    }

    fn write(&mut self, path: &Path, content: &[u8]) -> io::Result<()> {
        let destination = if self.fail_write {
            path.join("missing")
        } else {
            path.to_path_buf()
        };
        LocalFiles.write(&destination, content)
    }
}

struct Scripted {
    device_url: String,
    access_url: String,
    device_replies: VecDeque<ScriptedReply>,
    access_replies: VecDeque<ScriptedReply>,
    requests: Vec<Value>,
}

impl Transport for Scripted {
    fn post_json<'call>(
        &'call mut self,
        post: Post<'call>,
    ) -> impl Future<Output = Result<Reply, TransportFailure>> + Send + 'call {
        let reply = if post.url == self.device_url.as_str() {
            self.device_replies.pop_front()
        } else if post.url == self.access_url.as_str() {
            self.access_replies.pop_front()
        } else {
            None
        };
        self.requests.push(json!({
            "url": post.url,
            "headers": post.headers,
            "body": serde_json::from_slice::<Value>(&post.body).unwrap_or(Value::Null),
        }));
        ready(
            reply.map_or(Err(TransportFailure::Unavailable), |scripted| {
                Ok(Reply {
                    status: scripted.status,
                    body: serde_json::to_vec(&scripted.body).unwrap_or_default(),
                })
            }),
        )
    }
}

#[derive(Default)]
struct Recorder {
    sleeps: Vec<Duration>,
}

impl Sleeper for Recorder {
    fn sleep(&mut self, duration: Duration) -> impl Future<Output = ()> + Send {
        self.sleeps.push(duration);
        ready(())
    }
}

#[derive(Default)]
struct Capture {
    messages: Vec<String>,
}

impl OperatorOutput for Capture {
    fn instructions(&mut self, message: &str) {
        self.messages.push(message.to_owned());
    }
}

type Fixture = Authenticator<Files, Recorder, Scripted, Capture>;

fn fixture(input: &Input, directory: &Path) -> Result<Fixture, FixtureError> {
    let mut settings = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("")?
    .sections
    .copilot_device;
    settings.token_dir = directory.display().to_string();
    if let Some(content) = &input.access_token_file {
        std::fs::write(directory.join(&settings.access_token_file), content)?;
    }
    let transport = Scripted {
        device_url: settings.device_code_url.clone(),
        access_url: settings.access_token_url.clone(),
        device_replies: input.device_replies.iter().cloned().collect(),
        access_replies: input.access_replies.iter().cloned().collect(),
        requests: Vec::new(),
    };
    let files = Files {
        fail_write: input.fail_write,
    };
    Ok(Authenticator::new(
        settings,
        files,
        Recorder::default(),
        transport,
        Capture::default(),
    )?)
}

fn run_fixture(input: &Input) -> Result<Value, FixtureError> {
    let directory = tempfile::tempdir()?;
    let mut authenticator = fixture(input, directory.path())?;
    let runtime = tokio::runtime::Builder::new_current_thread().build()?;
    let outcome = runtime.block_on(authenticator.get_access_token());
    let (result, error_status) = match &outcome {
        Ok(token) => (Value::String(token.expose_secret().to_owned()), Value::Null),
        Err(error) => (
            Value::Null,
            error.status().map_or(Value::Null, |status| json!(status)),
        ),
    };
    let file = std::fs::read_to_string(&authenticator.access_token_path)
        .map_or(Value::Null, Value::String);
    let sleeps: Vec<u64> = authenticator
        .sleeper
        .sleeps
        .iter()
        .map(Duration::as_secs)
        .collect();
    Ok(json!({
        "result": result,
        "error_status": error_status,
        "file": file,
        "requests": authenticator.transport.requests,
        "instructions": authenticator.output.messages,
        "sleeps": sleeps,
    }))
}

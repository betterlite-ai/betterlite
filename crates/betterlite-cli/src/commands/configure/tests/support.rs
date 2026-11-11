#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::State;
use axum::http::{StatusCode, Uri};
use axum::response::{IntoResponse as _, Response};
use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::settings::JsonValue;
use betterlite_logging::private_json::files::PrivateFiles;
use clap::Parser as _;
use secrecy::SecretString;
use serde::Serialize;
use soft_canonicalize::soft_canonicalize;
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use crate::agent_setup::error::AgentSetupError;
use crate::agent_setup::prompt::Prompter;
use crate::agent_setup_profiles::locks::SystemClock;
use crate::agent_setup_profiles::{SavedSetup, Target, save_setup, settings_path_for};
use crate::cli_context::CliContext;
use crate::codex_settings::version::CodexVersion;
use crate::commands::configure::args::{
    ConfigureArgs, ConnectionFlags, GroupConnection, ReconfigureArgs, UnconfigureArgs,
    UnconfigureTarget,
};
use crate::commands::configure::error::ConfigureCommandError;
use crate::commands::configure::{AgentSetupCommand, SetupPorts, run};

pub(crate) const SAFE_CODEX: &str = "codex-cli 0.129.0";
pub(crate) const SAVED_KEY: &str = "sk-saved";

#[derive(clap::Parser)]
pub(crate) struct TestCli {
    #[command(subcommand)]
    pub(crate) command: TestCommand,
}

#[derive(clap::Subcommand)]
pub(crate) enum TestCommand {
    Configure(ConfigureArgs),
    Reconfigure(ReconfigureArgs),
    Unconfigure(UnconfigureArgs),
}

pub(crate) fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

pub(crate) fn context(base_url: &str, api_key: Option<&str>) -> CliContext {
    CliContext {
        base_url: base_url.to_owned(),
        base_url_explicit: true,
        api_key: api_key.map(secret),
        api_key_from_token_file: false,
    }
}

pub(crate) fn implicit_context(base_url: &str, api_key: Option<&str>) -> CliContext {
    let mut result = context(base_url, api_key);
    result.base_url_explicit = false;
    result
}

pub(crate) fn token_context(base_url: &str, api_key: &str) -> CliContext {
    let mut result = context(base_url, Some(api_key));
    result.api_key_from_token_file = true;
    result
}

pub(crate) fn group_connection(
    api_key: Option<&str>,
    gateway_url: Option<&str>,
) -> GroupConnection {
    GroupConnection {
        api_key: api_key.map(secret),
        gateway_url: gateway_url.map(str::to_owned),
    }
}

impl From<GroupConnection> for ConnectionFlags {
    fn from(group: GroupConnection) -> Self {
        Self {
            api_key: group.api_key,
            gateway_url: group.gateway_url,
        }
    }
}

pub(crate) fn group_configure() -> ConfigureArgs {
    ConfigureArgs {
        connection: group_connection(None, None),
        command: None,
    }
}

pub(crate) fn target_configure(
    target: Target,
    base_url: &str,
    api_key: &str,
    model: &str,
) -> ConfigureArgs {
    let name = match target {
        Target::Claude => "claude",
        Target::Codex => "codex",
    };
    let parsed = TestCli::try_parse_from([
        "lite",
        "configure",
        name,
        "--api-key",
        api_key,
        "--gateway-url",
        base_url,
        "--model",
        model,
    ])
    .unwrap();
    let TestCommand::Configure(args) = parsed.command else {
        panic!("expected the configure command");
    };
    args
}

pub(crate) const fn unconfigure_claude() -> UnconfigureArgs {
    UnconfigureArgs {
        forget: false,
        command: Some(UnconfigureTarget::Claude { forget: false }),
    }
}

pub(crate) struct FakeEnvironment {
    pub(crate) home: Option<PathBuf>,
}

impl FakeEnvironment {
    pub(crate) fn with_home(home: &Path) -> Self {
        Self {
            home: Some(home.to_owned()),
        }
    }
}

impl EnvironmentLookup for FakeEnvironment {
    fn lookup(&self, _name: &str) -> Option<String> {
        None
    }

    fn system_home(&self) -> Option<PathBuf> {
        self.home.clone()
    }
}

struct MockState {
    bodies: Mutex<VecDeque<String>>,
    requests: Arc<Mutex<Vec<String>>>,
}

async fn answer(State(state): State<Arc<MockState>>, uri: Uri) -> Response {
    state.requests.lock().unwrap().push(uri.path().to_owned());
    let body = state.bodies.lock().unwrap().pop_front();
    body.map_or_else(
        || StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        |response_body| (StatusCode::OK, response_body).into_response(),
    )
}

pub(crate) struct MockProxy {
    pub(crate) base_url: String,
    requests: Arc<Mutex<Vec<String>>>,
    _tasks: JoinSet<Result<(), io::Error>>,
}

impl MockProxy {
    pub(crate) async fn start(bodies: Vec<String>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let state = Arc::new(MockState {
            bodies: Mutex::new(VecDeque::from(bodies)),
            requests: Arc::clone(&requests),
        });
        let app = Router::new().fallback(answer).with_state(state);
        let mut tasks = JoinSet::new();
        tasks.spawn(async move { axum::serve(listener, app).await });
        Self {
            base_url,
            requests,
            _tasks: tasks,
        }
    }

    pub(crate) fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

#[derive(Serialize)]
struct ListingBody {
    data: Vec<ModelBody>,
}

#[derive(Serialize)]
struct ModelBody {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_model: Option<String>,
}

pub(crate) fn listing(models: &[(&str, Option<&str>)]) -> String {
    let data: Vec<ModelBody> = models
        .iter()
        .map(|(id, source)| ModelBody {
            id: (*id).to_owned(),
            source_model: (*source).map(str::to_owned),
        })
        .collect();
    serde_json::to_string(&ListingBody { data }).unwrap()
}

pub(crate) enum Reply {
    Targets(Vec<Target>),
    Model(String),
    Url(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Call {
    Targets {
        message: String,
        defaults: Vec<Target>,
        on_empty: String,
    },
    Model {
        message: String,
        choices: Vec<String>,
        selected: usize,
    },
    Url {
        message: String,
        default: String,
    },
    Secret {
        message: String,
    },
}

pub(crate) struct ScriptedPrompter {
    replies: VecDeque<Reply>,
    pub(crate) calls: Vec<Call>,
}

impl ScriptedPrompter {
    fn answer(&mut self, call: Call) -> Result<Reply, AgentSetupError> {
        self.calls.push(call);
        self.replies
            .pop_front()
            .ok_or(AgentSetupError::ListingTaskStopped)
    }
}

impl Prompter for ScriptedPrompter {
    fn pick_targets(
        &mut self,
        message: &str,
        defaults: &[Target],
        on_empty: &str,
    ) -> Result<Vec<Target>, AgentSetupError> {
        let call = Call::Targets {
            message: message.to_owned(),
            defaults: defaults.to_vec(),
            on_empty: on_empty.to_owned(),
        };
        match self.answer(call)? {
            Reply::Targets(targets) => Ok(targets),
            _unused @ (Reply::Model(_) | Reply::Url(_)) => Err(AgentSetupError::ListingTaskStopped),
        }
    }

    fn pick_model(
        &mut self,
        message: &str,
        choices: &[String],
        selected: usize,
    ) -> Result<String, AgentSetupError> {
        let call = Call::Model {
            message: message.to_owned(),
            choices: choices.to_vec(),
            selected,
        };
        match self.answer(call)? {
            Reply::Model(model) => Ok(model),
            _unused @ (Reply::Targets(_) | Reply::Url(_)) => {
                Err(AgentSetupError::ListingTaskStopped)
            }
        }
    }

    fn prompt_url(&mut self, message: &str, default: &str) -> Result<String, AgentSetupError> {
        let call = Call::Url {
            message: message.to_owned(),
            default: default.to_owned(),
        };
        match self.answer(call)? {
            Reply::Url(url) => Ok(url),
            _unused @ (Reply::Targets(_) | Reply::Model(_)) => {
                Err(AgentSetupError::ListingTaskStopped)
            }
        }
    }

    fn prompt_secret(&mut self, message: &str) -> Result<SecretString, AgentSetupError> {
        // every test run keeps the saved key, which is what an empty entry at this prompt means
        self.calls.push(Call::Secret {
            message: message.to_owned(),
        });
        Ok(SecretString::from(String::new()))
    }
}

pub(crate) struct Harness {
    pub(crate) context: CliContext,
    pub(crate) environment: FakeEnvironment,
    pub(crate) prompter: ScriptedPrompter,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
    pub(crate) stdin_is_terminal: bool,
    pub(crate) codex_version: CodexVersion<'static>,
}

macro_rules! setup_runner {
    ($name:ident, $arguments:ty, $command:path) => {
        pub(crate) async fn $name(
            &mut self,
            args: $arguments,
        ) -> Result<(), ConfigureCommandError> {
            let files = PrivateFiles::default();
            let clock = SystemClock;
            let mut ports = SetupPorts {
                context: &self.context,
                environment: &self.environment,
                files: &files,
                prompter: &mut self.prompter,
                stdin_is_terminal: self.stdin_is_terminal,
                clock: &clock,
                codex_version: self.codex_version,
                stdout: &mut self.stdout,
                stderr: &mut self.stderr,
            };
            run($command(args), &mut ports).await
        }
    };
}

impl Harness {
    pub(crate) fn new(environment: FakeEnvironment, context: CliContext) -> Self {
        Self {
            context,
            environment,
            prompter: ScriptedPrompter {
                replies: VecDeque::new(),
                calls: Vec::new(),
            },
            stdout: Vec::new(),
            stderr: Vec::new(),
            stdin_is_terminal: false,
            codex_version: CodexVersion::Reported(Some(SAFE_CODEX)),
        }
    }

    setup_runner!(configure, ConfigureArgs, AgentSetupCommand::Configure);
    setup_runner!(reconfigure, ReconfigureArgs, AgentSetupCommand::Reconfigure);
    setup_runner!(unconfigure, UnconfigureArgs, AgentSetupCommand::Unconfigure);

    pub(crate) fn reply(&mut self, reply: Reply) {
        self.prompter.replies.push_back(reply);
    }

    pub(crate) fn stdout_text(&self) -> String {
        String::from_utf8(self.stdout.clone()).unwrap()
    }

    pub(crate) fn stderr_text(&self) -> String {
        String::from_utf8(self.stderr.clone()).unwrap()
    }
}

pub(crate) async fn save_profile(
    target: Target,
    environment: &FakeEnvironment,
    base_url: &str,
    model: Option<&str>,
) {
    let path = settings_path_for(target, environment).unwrap();
    let resolved = soft_canonicalize(&path).unwrap();
    let saved = SavedSetup::new(
        target,
        resolved.to_str().unwrap().to_owned(),
        base_url.to_owned(),
        secret(SAVED_KEY),
        model.map(str::to_owned),
    )
    .unwrap();
    save_setup(&saved, environment).await.unwrap();
}

pub(crate) fn claude_settings(environment: &FakeEnvironment) -> BTreeMap<String, JsonValue> {
    let path = settings_path_for(Target::Claude, environment).unwrap();
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

pub(crate) fn write_claude_settings(
    environment: &FakeEnvironment,
    settings: &BTreeMap<String, JsonValue>,
) {
    let path = settings_path_for(Target::Claude, environment).unwrap();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&path, serde_json::to_vec(settings).unwrap()).unwrap();
}

pub(crate) fn claude_env(settings: &BTreeMap<String, JsonValue>, key: &str) -> Option<String> {
    let env = settings.get("env")?;
    env.get(key)?.as_str().map(str::to_owned)
}

pub(crate) fn codex_config(environment: &FakeEnvironment) -> String {
    let path = settings_path_for(Target::Codex, environment).unwrap();
    fs::read_to_string(path).unwrap()
}

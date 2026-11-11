use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::State;
use axum::http::header::{CONTENT_TYPE, LOCATION};
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse as _, Response};
use betterlite_config::loader::EnvironmentLookup;
use secrecy::SecretString;
use serde::Serialize;
use soft_canonicalize::soft_canonicalize;
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use crate::agent_setup::error::AgentSetupError;
use crate::agent_setup::prepare::SetupMode;
use crate::agent_setup::prompt::Prompter;
use crate::agent_setup::{ConfigureTargets, configure_targets};
use crate::agent_setup_profiles::{SavedSetup, Target, save_setup};
use crate::cli_context::CliContext;
use crate::codex_settings::version::CodexVersion;
use crate::pi::ListedModel;

pub(crate) const APPLY: SetupMode = SetupMode {
    interactive: false,
    edit_connection: false,
};

pub(crate) const WIZARD: SetupMode = SetupMode {
    interactive: true,
    edit_connection: false,
};

pub(crate) const EDIT: SetupMode = SetupMode {
    interactive: false,
    edit_connection: true,
};

pub(crate) const EDIT_WIZARD: SetupMode = SetupMode {
    interactive: true,
    edit_connection: true,
};

pub(crate) const SAFE_CODEX: &str = "codex-cli 0.129.0";

pub(crate) fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

pub(crate) fn context(base_url: &str, key: Option<&str>) -> CliContext {
    CliContext {
        base_url: base_url.to_owned(),
        base_url_explicit: true,
        api_key: key.map(secret),
        api_key_from_token_file: false,
    }
}

pub(crate) fn implicit_context(base_url: &str, key: Option<&str>) -> CliContext {
    let mut result = context(base_url, key);
    result.base_url_explicit = false;
    result
}

pub(crate) fn token_file_context(base_url: &str, key: Option<&str>) -> CliContext {
    let mut result = context(base_url, key);
    result.api_key_from_token_file = true;
    result
}

pub(crate) struct FakeEnvironment {
    pub(crate) variables: BTreeMap<String, String>,
    pub(crate) home: Option<PathBuf>,
}

impl FakeEnvironment {
    pub(crate) fn with_home(home: &Path) -> Self {
        Self {
            variables: BTreeMap::new(),
            home: Some(home.to_owned()),
        }
    }

    pub(crate) fn without_home() -> Self {
        Self {
            variables: BTreeMap::new(),
            home: None,
        }
    }

    pub(crate) fn with_variable(mut self, name: &str, value: &str) -> Self {
        self.variables.insert(name.to_owned(), value.to_owned());
        self
    }
}

impl EnvironmentLookup for FakeEnvironment {
    fn lookup(&self, name: &str) -> Option<String> {
        self.variables.get(name).cloned()
    }

    fn system_home(&self) -> Option<PathBuf> {
        self.home.clone()
    }
}

#[derive(Clone)]
pub(crate) enum Answer {
    Status(u16, String),
    Redirect(String),
}

#[derive(Clone)]
pub(crate) struct Recorded {
    pub(crate) path: String,
    headers: Vec<(String, String)>,
}

impl Recorded {
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header, _value)| header.eq_ignore_ascii_case(name))
            .map(|(_header, value)| value.as_str())
    }
}

struct MockState {
    answers: Mutex<VecDeque<Answer>>,
    requests: Arc<Mutex<Vec<Recorded>>>,
}

async fn answer(State(state): State<Arc<MockState>>, uri: Uri, headers: HeaderMap) -> Response {
    state.requests.lock().unwrap().push(Recorded {
        path: uri.path().to_owned(),
        headers: headers
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_owned(),
                    value.to_str().unwrap_or_default().to_owned(),
                )
            })
            .collect(),
    });
    let next = state.answers.lock().unwrap().pop_front();
    match next {
        Some(Answer::Status(status, body)) => {
            let code = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            let mut response = (code, body).into_response();
            response
                .headers_mut()
                .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
            response
        }
        Some(Answer::Redirect(location)) => {
            let mut response = StatusCode::FOUND.into_response();
            response.headers_mut().insert(
                LOCATION,
                HeaderValue::from_str(&location).unwrap_or(HeaderValue::from_static("/")),
            );
            response
        }
        None => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

pub(crate) struct MockProxy {
    pub(crate) base_url: String,
    requests: Arc<Mutex<Vec<Recorded>>>,
    _tasks: JoinSet<Result<(), io::Error>>,
}

impl MockProxy {
    pub(crate) async fn start(answers: Vec<Answer>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let state = Arc::new(MockState {
            answers: Mutex::new(VecDeque::from(answers)),
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

    pub(crate) fn requests(&self) -> Vec<Recorded> {
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

impl From<ListedModel> for ModelBody {
    fn from(model: ListedModel) -> Self {
        Self {
            id: model.id,
            source_model: model.source_model,
        }
    }
}

pub(crate) fn listing(models: &[(&str, Option<&str>)]) -> String {
    let data: Vec<ModelBody> = listed(models).into_iter().map(ModelBody::from).collect();
    serde_json::to_string(&ListingBody { data }).unwrap()
}

pub(crate) fn listed(models: &[(&str, Option<&str>)]) -> Vec<ListedModel> {
    models
        .iter()
        .map(|(id, source)| ListedModel {
            id: (*id).to_owned(),
            source_model: (*source).map(str::to_owned),
        })
        .collect()
}

pub(crate) enum Reply {
    Targets(Vec<Target>),
    Model(String),
    Url(String),
    Secret(String),
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
    pub(crate) after_url: Option<Box<dyn FnMut() + Send>>,
}

impl ScriptedPrompter {
    pub(crate) fn new(replies: Vec<Reply>) -> Self {
        Self {
            replies: VecDeque::from(replies),
            calls: Vec::new(),
            after_url: None,
        }
    }

    fn answer(&mut self, call: Call) -> Result<Reply, AgentSetupError> {
        self.calls.push(call);
        self.replies
            .pop_front()
            .ok_or(AgentSetupError::ListingTaskStopped)
    }
}

fn model_call(message: &str, choices: &[String], selected: usize) -> Call {
    Call::Model {
        message: message.to_owned(),
        choices: choices.to_vec(),
        selected,
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
            _unused @ (Reply::Model(_) | Reply::Url(_) | Reply::Secret(_)) => {
                Err(AgentSetupError::ListingTaskStopped)
            }
        }
    }

    fn pick_model(
        &mut self,
        message: &str,
        choices: &[String],
        selected: usize,
    ) -> Result<String, AgentSetupError> {
        let call = model_call(message, choices, selected);
        match self.answer(call)? {
            Reply::Model(model) => Ok(model),
            _unused @ (Reply::Targets(_) | Reply::Url(_) | Reply::Secret(_)) => {
                Err(AgentSetupError::ListingTaskStopped)
            }
        }
    }

    fn prompt_url(&mut self, message: &str, default: &str) -> Result<String, AgentSetupError> {
        let call = Call::Url {
            message: message.to_owned(),
            default: default.to_owned(),
        };
        let reply = self.answer(call);
        if let Some(callback) = self.after_url.as_mut() {
            callback();
        }
        match reply? {
            Reply::Url(url) => Ok(url),
            _unused @ (Reply::Targets(_) | Reply::Model(_) | Reply::Secret(_)) => {
                Err(AgentSetupError::ListingTaskStopped)
            }
        }
    }

    fn prompt_secret(&mut self, message: &str) -> Result<SecretString, AgentSetupError> {
        let call = Call::Secret {
            message: message.to_owned(),
        };
        match self.answer(call)? {
            Reply::Secret(value) => Ok(SecretString::from(value)),
            _unused @ (Reply::Targets(_) | Reply::Model(_) | Reply::Url(_)) => {
                Err(AgentSetupError::ListingTaskStopped)
            }
        }
    }
}

pub(crate) struct Run<'run> {
    pub(crate) context: &'run CliContext,
    pub(crate) targets: Vec<Target>,
    pub(crate) model: Option<&'run str>,
    pub(crate) default_model: bool,
    pub(crate) mode: SetupMode,
    pub(crate) environment: &'run FakeEnvironment,
    pub(crate) stdin_is_terminal: bool,
    pub(crate) codex_version: CodexVersion<'run>,
}

impl Run<'_> {
    pub(crate) async fn execute(
        &self,
        prompter: &mut ScriptedPrompter,
        stdout: &mut Vec<u8>,
        stderr: &mut Vec<u8>,
    ) -> Result<(), AgentSetupError> {
        configure_targets(ConfigureTargets {
            context: self.context,
            targets: &self.targets,
            model: self.model,
            default_model: self.default_model,
            mode: self.mode,
            environment: self.environment,
            prompter,
            stdin_is_terminal: self.stdin_is_terminal,
            codex_version: self.codex_version,
            stdout,
            stderr,
        })
        .await
    }
}

pub(crate) async fn save(profile: &SavedSetup, environment: &FakeEnvironment) {
    save_setup(profile, environment).await.unwrap();
}

pub(crate) fn saved_setup(
    target: Target,
    settings_path: &Path,
    base_url: &str,
    api_key: &str,
    model: Option<&str>,
) -> SavedSetup {
    let resolved = soft_canonicalize(settings_path).unwrap();
    SavedSetup::new(
        target,
        resolved.to_str().unwrap().to_owned(),
        base_url.to_owned(),
        secret(api_key),
        model.map(str::to_owned),
    )
    .unwrap()
}

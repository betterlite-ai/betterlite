use crate::helpers::{Helper, HelperError, HelperResult};
use betterlite_cli::agent_launch::{EnvVars, Platform, Terminal};
use betterlite_cli::agent_run::error::AgentRunError;
use betterlite_cli::agent_run::pipeline::{RunAgentRequest, run_agent};
use betterlite_cli::agent_run::prepare::{CodexPreparer, PiPreparer, Preparers};
use betterlite_cli::agent_run::sync::{AgentModelSync, SyncContext};
use betterlite_logging::private_json::files::LocalFiles;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::fmt;
use std::fs;
use std::path::Path;
use std::sync::{Arc, PoisonError};
mod fakes;
use self::fakes::{
    Debug, DebugScript, Http, Installer, Launcher, Probe, Recorder, Resolver, SilentTerminal,
};
pub(crate) const INSTALL_COMMAND: &str = "/usr/local/bin/betterlite statusline";
const BASE_URL: &str = "http://gateway.test:4000/";
const API_KEY: &str = "sk-test";
const DEFAULT_INSTRUCTIONS: &str = "You are Codex.";
const EMPTY_CATALOG: &str = "{\"models\":[]}";
const FAILED_CATALOG: &str = "error: invalid catalog";
pub(crate) struct Op;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    steps: Vec<Step>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    agent: Agent,
    scenario: Scenario,
    command: Vec<String>,
    binary: Option<String>,
    #[serde(default)]
    models: Vec<Value>,
    #[serde(default)]
    stock: Option<String>,
    #[serde(default)]
    instructions: Option<String>,
}
#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Agent {
    Pi,
    Codex,
    Opencode,
}
#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum Scenario {
    Launch,
    Skip,
    ReadBackFailure,
    Hooks,
    ExistingConfig,
    MissingBinary,
}
impl Helper for Op {
    type Input = Input;
    fn run(input: &Self::Input) -> HelperResult {
        let mut steps = Vec::new();
        for step in &input.steps {
            steps.push(run_step(step)?);
        }
        Ok(json!({"steps": steps}))
    }
}
struct Setup {
    directory: tempfile::TempDir,
    env: EnvVars,
    root: String,
}
impl Setup {
    fn new(step: &Step) -> Result<Self, Box<HelperError>> {
        let directory = tempfile::tempdir().map_err(failure)?;
        let mut env = EnvVars::new();
        env.insert("KEEP".to_owned(), "1".to_owned());
        if step.scenario == Scenario::ExistingConfig {
            env.insert(
                "OPENCODE_CONFIG_CONTENT".to_owned(),
                "{\"provider\":{}}".to_owned(),
            );
        }
        let home = directory.path().join("home");
        env.extend([
            ("HOME".to_owned(), path_text(&home)?),
            (
                "CODEX_HOME".to_owned(),
                path_text(&directory.path().join("codex"))?,
            ),
            (
                "PI_CODING_AGENT_DIR".to_owned(),
                path_text(&directory.path().join("pi"))?,
            ),
        ]);
        if step.scenario == Scenario::Hooks {
            let codex_home = directory.path().join("codex");
            fs::create_dir_all(&codex_home).map_err(failure)?;
            fs::write(codex_home.join("config.toml"), "hooks.Stop = []\n").map_err(failure)?;
        }
        Ok(Self {
            root: path_text(directory.path())?,
            directory,
            env,
        })
    }
}
fn run_step(step: &Step) -> HelperResult {
    let setup = Setup::new(step)?;
    let outcome = execute(step, &setup);
    Ok(report(
        &outcome.result,
        &outcome.launcher,
        &outcome.messages,
        &setup,
    ))
}
struct Fakes {
    http: Http,
    debug: Debug,
    installer: Installer,
    launcher: Launcher,
    terminal: Arc<SilentTerminal>,
    resolver: Resolver,
    probe: Probe,
}
impl Fakes {
    fn new(step: &Step) -> Self {
        Self {
            http: Http {
                rows: step.models.clone(),
            },
            debug: Debug::new(debug_answers(step)),
            installer: Installer,
            launcher: Launcher::new(),
            terminal: Arc::new(SilentTerminal),
            resolver: Resolver(step.binary.clone()),
            probe: Probe,
        }
    }
}
struct Outcome {
    result: Result<i32, AgentRunError>,
    launcher: Launcher,
    messages: Recorder,
}
fn execute(step: &Step, setup: &Setup) -> Outcome {
    let fakes = Fakes::new(step);
    let files = LocalFiles;
    let pi = PiPreparer {
        get: &fakes.http,
        files: &files,
    };
    let codex = CodexPreparer {
        install: &fakes.installer,
    };
    let preparers = Preparers {
        pi: &pi,
        codex: &codex,
    };
    let sync = model_sync(step, &fakes, &files);
    let controlling: &dyn Terminal = fakes.terminal.as_ref();
    let api_key = SecretString::from(API_KEY);
    let mut messages = Recorder::default();
    let request = RunAgentRequest {
        base_url: BASE_URL,
        api_key: &api_key,
        command: &step.command,
        skip_verify: step.scenario == Scenario::Skip,
        base_env: &setup.env,
        home: None,
        resolver: &fakes.resolver,
        probe: &fakes.probe,
        sync: &sync,
        preparers: &preparers,
        launcher: &fakes.launcher,
        platform: Platform::Unix,
        terminal: (step.scenario != Scenario::MissingBinary).then_some(controlling),
        messages: &mut messages,
    };
    let result = run_agent(request);
    Outcome {
        result,
        launcher: fakes.launcher,
        messages,
    }
}
fn model_sync<'request>(
    step: &'request Step,
    fakes: &'request Fakes,
    files: &'request LocalFiles,
) -> AgentModelSync<'request> {
    AgentModelSync {
        context: SyncContext {
            get: &fakes.http,
            debug: &fakes.debug,
            files,
            home: None,
            instructions: Some(step.instructions.as_deref().unwrap_or(DEFAULT_INSTRUCTIONS)),
        },
    }
}
fn debug_answers(step: &Step) -> Vec<DebugScript> {
    if step.agent != Agent::Codex || step.scenario == Scenario::Skip {
        return Vec::new();
    }
    let stock = step
        .stock
        .clone()
        .unwrap_or_else(|| EMPTY_CATALOG.to_owned());
    let first = DebugScript {
        status: 0_i32,
        stdout: stock,
        stderr: String::new(),
    };
    let second = if step.scenario == Scenario::ReadBackFailure {
        DebugScript {
            status: 2_i32,
            stdout: String::new(),
            stderr: FAILED_CATALOG.to_owned(),
        }
    } else {
        DebugScript {
            status: 0_i32,
            stdout: EMPTY_CATALOG.to_owned(),
            stderr: String::new(),
        }
    };
    vec![first, second]
}
fn report(
    result: &Result<i32, AgentRunError>,
    launcher: &Launcher,
    messages: &Recorder,
    setup: &Setup,
) -> Value {
    let root = setup.root.as_str();
    json!({
        "error": match result { Ok(_code) => Value::Null, Err(error) => error_json(error) },
        "warnings": messages.warns.iter().map(|value| value.replace(root, "<tmp>")).collect::<Vec<String>>(),
        "notices": messages.notices.iter().map(|value| value.replace(root, "<tmp>")).collect::<Vec<String>>(),
        "launch": launch_json(launcher, root),
        "catalog": file_json(&setup.directory.path().join("codex/litellm-models.json")),
        "pi_models": file_json(&setup.directory.path().join("pi/models.json")),
    })
}
fn failure<M>(message: M) -> Box<HelperError>
where
    M: fmt::Display,
{
    Box::new(HelperError {
        class: "AgentExecution".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_string(),
    })
}
fn path_text(path: &Path) -> Result<String, Box<HelperError>> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| failure("a scripted path is not valid UTF-8"))
}
fn error_json(error: &AgentRunError) -> Value {
    if let AgentRunError::MissingBinary { hint, .. } = error {
        return json!({
            "kind": "missing_binary",
            "docs": hint.strip_prefix(" Install it first: "),
        });
    }
    json!({"kind": "failed", "docs": null})
}
fn env_json(env: &EnvVars, root: &str) -> Value {
    let mut fields = Map::new();
    for (name, value) in env {
        let text = value.replace(root, "<tmp>");
        let entry = if name == "OPENCODE_CONFIG_CONTENT" {
            serde_json::from_str(text.as_str()).unwrap_or(Value::String(text))
        } else {
            Value::String(text)
        };
        fields.insert(name.clone(), entry);
    }
    Value::Object(fields)
}
fn launch_json(launcher: &Launcher, root: &str) -> Value {
    let launch = launcher
        .calls
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .first()
        .cloned();
    let Some(call) = launch else {
        return Value::Null;
    };
    json!({
        "binary": call.path.replace(root, "<tmp>"),
        "argv": call.args.iter().map(|argument| argument.replace(root, "<tmp>")).collect::<Vec<String>>(),
        "env": env_json(&call.env, root),
    })
}
fn file_json(path: &Path) -> Value {
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .unwrap_or(Value::Null)
}

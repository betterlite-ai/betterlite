use crate::helpers::{Helper, HelperError, HelperResult};
use betterlite_cli::agent_launch::{EnvVars, Platform, Terminal};
use betterlite_cli::agent_run::error::AgentRunError;
use betterlite_cli::agent_run::pipeline::{RunAgentRequest, run_agent};
use betterlite_cli::agent_run::prepare::{CodexPreparer, PiPreparer, Preparers};
use betterlite_cli::agent_run::sync::{AgentModelSync, SyncContext};
use betterlite_logging::private_json::files::LocalFiles;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::fs;
use std::sync::{Arc, Mutex};
mod fakes;
mod report;
use self::fakes::{
    Debug, DebugScript, Http, Installer, Launcher, Probe, Recorder, Resolver, SilentTerminal,
};
use self::report::{failure, path_text, report};
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
    let fakes = Fakes::new(step);
    let mut messages = Recorder::default();
    let result = execute(step, &setup, &fakes, &mut messages);
    Ok(report(&result, &fakes.launcher, &messages, &setup))
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
            debug: Debug {
                answers: Mutex::new(VecDeque::from(debug_answers(step))),
            },
            installer: Installer,
            launcher: Launcher {
                calls: Mutex::new(Vec::new()),
            },
            terminal: Arc::new(SilentTerminal),
            resolver: Resolver(step.binary.clone()),
            probe: Probe,
        }
    }
}
fn execute(
    step: &Step,
    setup: &Setup,
    fakes: &Fakes,
    messages: &mut Recorder,
) -> Result<i32, AgentRunError> {
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
    let sync = AgentModelSync {
        context: SyncContext {
            get: &fakes.http,
            debug: &fakes.debug,
            files: &files,
            home: None,
            instructions: DEFAULT_INSTRUCTIONS,
        },
    };
    let controlling: &dyn Terminal = fakes.terminal.as_ref();
    let api_key = SecretString::from(API_KEY);
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
        messages,
    };
    run_agent(request)
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

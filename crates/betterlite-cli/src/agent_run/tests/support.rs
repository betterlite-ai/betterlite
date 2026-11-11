use std::collections::{BTreeMap, VecDeque};
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_logging::private_json::files::{FileAccess, LocalFiles, StagedTemp};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use crate::agent_launch::{EnvVars, KeyProbe, Platform, ProcessLauncher, Terminal, WindowsCommand};
use crate::agent_run::codex::{DebugFailure, DebugInvocation, DebugModels, DebugOutput};
use crate::agent_run::error::AgentRunError;
use crate::agent_run::messages::Messages;
use crate::agent_run::pipeline::{RunAgentRequest, run_agent};
use crate::agent_run::prepare::{PrepareRequest, Preparer, Preparers, StatuslineInstaller};
use crate::agent_run::resolver::BinaryResolver;
use crate::agent_run::sync::{ModelSync, SyncRequest};
use crate::claude_settings::error::ClaudeSettingsError;
use crate::opencode::{
    ModelListingRequest, ModelListingResponse, ModelListingTransport, ModelSyncResult,
};
use crate::pi::{ProxyGet, ProxyResponse};

pub(super) const API_KEY: &str = "sk-test";
pub(super) const BASE_URL: &str = "http://gateway.test:4000/";
pub(super) const CODEX_BINARY: &str = "/usr/local/bin/codex";

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(super) fn key() -> SecretString {
    SecretString::from(API_KEY)
}

pub(super) fn vars(pairs: &[(&str, &str)]) -> EnvVars {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn command(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| (*part).to_owned()).collect()
}

pub(super) struct FakeEnvironment {
    pub(super) variables: EnvVars,
    pub(super) home: Option<PathBuf>,
}

impl EnvironmentLookup for FakeEnvironment {
    fn lookup(&self, name: &str) -> Option<String> {
        self.variables.get(name).cloned()
    }

    fn system_home(&self) -> Option<PathBuf> {
        self.home.clone()
    }
}

pub(super) struct RecordingResolver {
    pub(super) binary: Option<String>,
    pub(super) asked: Mutex<Vec<String>>,
}

impl RecordingResolver {
    pub(super) fn returning(binary: Option<&str>) -> Self {
        Self {
            binary: binary.map(str::to_owned),
            asked: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn asked(&self) -> Vec<String> {
        locked(&self.asked).clone()
    }
}

impl BinaryResolver for RecordingResolver {
    fn resolve(&self, command: &str) -> Option<String> {
        locked(&self.asked).push(command.to_owned());
        self.binary.clone()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ProbeCall {
    pub(super) url: String,
    pub(super) key_matches: bool,
    pub(super) timeout: Duration,
}

pub(super) struct ScriptedProbe {
    pub(super) status: u16,
    pub(super) failure: Option<io::ErrorKind>,
    pub(super) calls: Mutex<Vec<ProbeCall>>,
}

impl ScriptedProbe {
    pub(super) fn returning(status: u16) -> Self {
        Self {
            status,
            failure: None,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn failing(kind: io::ErrorKind) -> Self {
        Self {
            status: 0,
            failure: Some(kind),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn calls(&self) -> Vec<ProbeCall> {
        locked(&self.calls).clone()
    }
}

impl KeyProbe for ScriptedProbe {
    fn status(&self, url: &str, api_key: &SecretString, timeout: Duration) -> io::Result<u16> {
        locked(&self.calls).push(ProbeCall {
            url: url.to_owned(),
            key_matches: api_key.expose_secret() == API_KEY,
            timeout,
        });
        self.failure
            .map_or(Ok(self.status), |kind| Err(io::Error::from(kind)))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct HttpCall {
    pub(super) url: String,
    pub(super) authorization: String,
    pub(super) timeout: Duration,
}

pub(super) enum HttpAnswer {
    Body { status: u16, body: Vec<u8> },
    Failure(io::ErrorKind),
}

pub(super) struct ScriptedHttp {
    answers: Mutex<VecDeque<HttpAnswer>>,
    calls: Mutex<Vec<HttpCall>>,
}

impl ScriptedHttp {
    pub(super) fn new(answers: Vec<HttpAnswer>) -> Self {
        Self {
            answers: Mutex::new(VecDeque::from(answers)),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn body(status: u16, body: &[u8]) -> HttpAnswer {
        HttpAnswer::Body {
            status,
            body: body.to_vec(),
        }
    }

    pub(super) fn calls(&self) -> Vec<HttpCall> {
        locked(&self.calls).clone()
    }

    fn answer(
        &self,
        url: &str,
        authorization: String,
        timeout: Duration,
    ) -> io::Result<(u16, Vec<u8>)> {
        locked(&self.calls).push(HttpCall {
            url: url.to_owned(),
            authorization,
            timeout,
        });
        let answer = locked(&self.answers).pop_front();
        match answer {
            Some(HttpAnswer::Body { status, body }) => Ok((status, body)),
            Some(HttpAnswer::Failure(kind)) => Err(io::Error::from(kind)),
            None => Err(io::Error::other("no scripted HTTP answer")),
        }
    }
}

fn authorization(headers: &[(String, SecretString)]) -> String {
    headers
        .iter()
        .find(|(name, _value)| name == "Authorization")
        .map_or_else(String::new, |(_name, value)| {
            value.expose_secret().to_owned()
        })
}

impl ProxyGet for ScriptedHttp {
    fn get(
        &self,
        url: &str,
        headers: &[(String, SecretString)],
        timeout: Duration,
    ) -> io::Result<ProxyResponse> {
        self.answer(url, authorization(headers), timeout)
            .map(|(status, body)| ProxyResponse { status, body })
    }
}

impl ModelListingTransport for ScriptedHttp {
    fn get(&self, request: &ModelListingRequest) -> io::Result<ModelListingResponse> {
        let (status, body) = self.answer(
            &request.url,
            request.authorization.expose_secret().to_owned(),
            request.timeout,
        )?;
        Ok(ModelListingResponse { status, body })
    }
}

pub(super) enum DebugAnswer {
    Exited {
        code: i32,
        stdout: String,
        stderr: String,
    },
    Timeout,
    Io(io::ErrorKind),
    Decode,
}

pub(super) struct ScriptedDebug {
    answers: Mutex<VecDeque<DebugAnswer>>,
    calls: Mutex<Vec<DebugInvocation>>,
}

impl ScriptedDebug {
    pub(super) fn new(answers: Vec<DebugAnswer>) -> Self {
        Self {
            answers: Mutex::new(VecDeque::from(answers)),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn printed(stdout: &str) -> DebugAnswer {
        DebugAnswer::Exited {
            code: 0,
            stdout: stdout.to_owned(),
            stderr: String::new(),
        }
    }

    pub(super) fn exited(code: i32, stderr: &str) -> DebugAnswer {
        DebugAnswer::Exited {
            code,
            stdout: String::new(),
            stderr: stderr.to_owned(),
        }
    }

    pub(super) fn calls(&self) -> Vec<DebugInvocation> {
        locked(&self.calls).clone()
    }
}

impl DebugModels for ScriptedDebug {
    fn run(&self, invocation: &DebugInvocation) -> Result<DebugOutput, DebugFailure> {
        locked(&self.calls).push(invocation.clone());
        let answer = locked(&self.answers).pop_front();
        match answer {
            Some(DebugAnswer::Exited {
                code,
                stdout,
                stderr,
            }) => Ok(DebugOutput {
                code,
                stdout,
                stderr,
            }),
            Some(DebugAnswer::Timeout) => Err(DebugFailure::Timeout),
            Some(DebugAnswer::Io(kind)) => Err(DebugFailure::Io(io::Error::from(kind))),
            Some(DebugAnswer::Decode) => Err(DebugFailure::Decode(
                String::from_utf8(vec![0xff]).unwrap_err(),
            )),
            None => Err(DebugFailure::Io(io::Error::other(
                "no scripted debug answer",
            ))),
        }
    }
}

#[derive(Default)]
pub(super) struct RecordingMessages {
    pub(super) warns: Mutex<Vec<String>>,
    pub(super) notices: Mutex<Vec<String>>,
}

impl RecordingMessages {
    pub(super) fn warns(&self) -> Vec<String> {
        locked(&self.warns).clone()
    }

    pub(super) fn notices(&self) -> Vec<String> {
        locked(&self.notices).clone()
    }
}

impl Messages for RecordingMessages {
    fn warn(&mut self, message: &str) -> io::Result<()> {
        locked(&self.warns).push(message.to_owned());
        Ok(())
    }

    fn notice(&mut self, message: &str) -> io::Result<()> {
        locked(&self.notices).push(message.to_owned());
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct LaunchCall {
    pub(super) path: String,
    pub(super) args: Vec<String>,
    pub(super) env: BTreeMap<String, String>,
}

pub(super) struct RecordingLauncher {
    pub(super) replacements: Mutex<Vec<LaunchCall>>,
    pub(super) spawns: Mutex<Vec<(WindowsCommand, BTreeMap<String, String>)>>,
    pub(super) result: Result<i32, io::ErrorKind>,
    pub(super) terminal_calls_at_launch: Mutex<Option<usize>>,
    pub(super) terminal: Option<Arc<RecordingTerminal>>,
}

impl RecordingLauncher {
    pub(super) fn exiting(code: i32) -> Self {
        Self {
            replacements: Mutex::new(Vec::new()),
            spawns: Mutex::new(Vec::new()),
            result: Ok(code),
            terminal_calls_at_launch: Mutex::new(None),
            terminal: None,
        }
    }

    pub(super) fn replacements(&self) -> Vec<LaunchCall> {
        locked(&self.replacements).clone()
    }

    pub(super) fn spawns(&self) -> Vec<(WindowsCommand, BTreeMap<String, String>)> {
        locked(&self.spawns).clone()
    }

    fn result_code(&self) -> io::Result<i32> {
        match self.result {
            Ok(code) => Ok(code),
            Err(kind) => Err(io::Error::from(kind)),
        }
    }

    fn note_launch(&self) {
        if let Some(terminal) = &self.terminal {
            *locked(&self.terminal_calls_at_launch) = Some(terminal.call_count());
        }
    }
}

impl ProcessLauncher for RecordingLauncher {
    fn replace(&self, path: &str, args: &[String], env: &EnvVars) -> io::Result<i32> {
        self.note_launch();
        locked(&self.replacements).push(LaunchCall {
            path: path.to_owned(),
            args: args.to_vec(),
            env: env.clone(),
        });
        self.result_code()
    }

    fn spawn(&self, command: &WindowsCommand, env: &EnvVars) -> io::Result<i32> {
        self.note_launch();
        locked(&self.spawns).push((command.clone(), env.clone()));
        self.result_code()
    }
}

pub(super) struct RecordingTerminal {
    pub(super) calls: Mutex<Vec<()>>,
    pub(super) failure: Option<io::ErrorKind>,
}

impl RecordingTerminal {
    pub(super) fn working() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            failure: None,
        }
    }

    pub(super) fn failing(kind: io::ErrorKind) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            failure: Some(kind),
        }
    }

    pub(super) fn call_count(&self) -> usize {
        locked(&self.calls).len()
    }
}

impl Terminal for RecordingTerminal {
    fn is_interactive(&self) -> bool {
        false
    }

    fn open_controlling(&self) -> io::Result<File> {
        Err(io::Error::from(io::ErrorKind::NotFound))
    }

    fn attach_stdin(&self, terminal: File) -> io::Result<()> {
        drop(terminal);
        Ok(())
    }

    fn restore_controlling_terminal(&self) -> io::Result<()> {
        locked(&self.calls).push(());
        self.failure
            .map_or(Ok(()), |kind| Err(io::Error::from(kind)))
    }
}

pub(super) struct ScriptedInstaller {
    pub(super) command: Result<String, String>,
    pub(super) calls: Mutex<Vec<()>>,
}

impl ScriptedInstaller {
    pub(super) fn installing(command: &str) -> Self {
        Self {
            command: Ok(command.to_owned()),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn failing(message: &str) -> Self {
        Self {
            command: Err(message.to_owned()),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn calls(&self) -> usize {
        locked(&self.calls).len()
    }
}

impl StatuslineInstaller for ScriptedInstaller {
    fn install(&self) -> Result<String, ClaudeSettingsError> {
        locked(&self.calls).push(());
        match &self.command {
            Ok(command) => Ok(command.clone()),
            Err(message) => Err(ClaudeSettingsError::CurrentExe {
                source: io::Error::other(message.clone()),
            }),
        }
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct ModelsFile {
    pub(super) providers: BTreeMap<String, ModelsProvider>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ModelsProvider {
    #[serde(rename = "baseUrl")]
    pub(super) base_url: String,
    pub(super) api: String,
    #[serde(rename = "apiKey")]
    pub(super) api_key: String,
    pub(super) models: Vec<ModelsEntry>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ModelsEntry {
    pub(super) id: String,
    #[serde(rename = "contextWindow", default)]
    pub(super) context_window: Option<i64>,
    #[serde(rename = "maxTokens", default)]
    pub(super) max_tokens: Option<i64>,
}

pub(super) fn read_models_file(path: &Path) -> ModelsFile {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

#[derive(Debug, Deserialize)]
pub(super) struct OpenCodeConfig {
    pub(super) provider: BTreeMap<String, OpenCodeProvider>,
}

#[derive(Debug, Deserialize)]
pub(super) struct OpenCodeProvider {
    pub(super) npm: String,
    pub(super) name: String,
    pub(super) options: OpenCodeOptions,
    pub(super) models: BTreeMap<String, OpenCodeModel>,
}

#[derive(Debug, Deserialize)]
pub(super) struct OpenCodeOptions {
    #[serde(rename = "baseURL")]
    pub(super) base_url: String,
    #[serde(rename = "apiKey")]
    pub(super) api_key: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct OpenCodeModel {
    pub(super) name: String,
    #[serde(default)]
    pub(super) limit: Option<OpenCodeLimit>,
}

#[derive(Debug, Deserialize)]
pub(super) struct OpenCodeLimit {
    pub(super) context: i64,
    pub(super) output: i64,
}

#[derive(Debug, Deserialize)]
pub(super) struct Catalog {
    pub(super) models: Vec<CatalogModel>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CatalogModel {
    pub(super) slug: String,
    pub(super) priority: i64,
    pub(super) visibility: String,
    pub(super) supported_in_api: bool,
    #[serde(default)]
    pub(super) display_name: Option<String>,
    #[serde(default)]
    pub(super) context_window: Option<i64>,
    #[serde(default)]
    pub(super) base_instructions: Option<String>,
    #[serde(default)]
    pub(super) upgrade: Option<CatalogUpgrade>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
pub(super) struct CatalogUpgrade {
    pub(super) model: String,
    #[serde(default)]
    pub(super) reason: Option<String>,
}

pub(super) fn read_catalog(path: &Path) -> Catalog {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

pub(super) fn files_in(directory: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_str().unwrap().to_owned())
        .collect();
    names.sort();
    names
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum FileFailure {
    Stage,
    Write,
    Replace,
    Parents,
}

pub(super) struct ScriptedFiles {
    inner: LocalFiles,
    failures: Vec<FileFailure>,
    staged: Mutex<Vec<PathBuf>>,
}

impl ScriptedFiles {
    pub(super) fn working() -> Self {
        Self {
            inner: LocalFiles,
            failures: Vec::new(),
            staged: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn failing(failure: FileFailure) -> Self {
        Self {
            inner: LocalFiles,
            failures: vec![failure],
            staged: Mutex::new(Vec::new()),
        }
    }

    fn fails(&self, failure: FileFailure) -> bool {
        self.failures.contains(&failure)
    }

    pub(super) fn staged_paths(&self) -> Vec<PathBuf> {
        locked(&self.staged).clone()
    }
}

fn scripted_failure() -> io::Error {
    io::Error::other("scripted filesystem failure")
}

impl FileAccess for ScriptedFiles {
    fn create_parents(&self, directory: &Path) -> io::Result<()> {
        if self.fails(FileFailure::Parents) {
            return Err(scripted_failure());
        }
        self.inner.create_parents(directory)
    }

    fn ensure_private_directory(&self, directory: &Path) -> io::Result<()> {
        self.inner.ensure_private_directory(directory)
    }

    fn directory_mode(&self, directory: &Path) -> io::Result<u32> {
        self.inner.directory_mode(directory)
    }

    fn set_mode(&self, path: &Path, mode: u32) -> io::Result<()> {
        self.inner.set_mode(path, mode)
    }

    fn create_staged(&self, directory: &Path, suffix: Option<&str>) -> io::Result<StagedTemp> {
        if self.fails(FileFailure::Stage) {
            return Err(scripted_failure());
        }
        self.inner.create_staged(directory, suffix)
    }

    fn write_staged(&self, staged: &mut StagedTemp, bytes: &[u8]) -> io::Result<()> {
        locked(&self.staged).push(staged.path.clone());
        if self.fails(FileFailure::Write) {
            return Err(scripted_failure());
        }
        self.inner.write_staged(staged, bytes)
    }

    fn replace(&self, staged: &Path, target: &Path) -> io::Result<()> {
        if self.fails(FileFailure::Replace) {
            return Err(scripted_failure());
        }
        self.inner.replace(staged, target)
    }

    fn remove(&self, path: &Path) -> io::Result<()> {
        self.inner.remove(path)
    }

    fn rewrite_existing(&self, target: &Path, bytes: &[u8]) -> io::Result<()> {
        self.inner.rewrite_existing(target, bytes)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SyncCall {
    pub(super) binary: String,
    pub(super) base_env: BTreeMap<String, String>,
    pub(super) base_url: String,
    pub(super) skip_verify: bool,
}

pub(super) struct RecordingSync {
    pub(super) result: Option<ModelSyncResult>,
    pub(super) calls: Mutex<Vec<SyncCall>>,
}

impl RecordingSync {
    pub(super) fn new(result: ModelSyncResult) -> Self {
        Self {
            result: Some(result),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn failing() -> Self {
        Self {
            result: None,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn calls(&self) -> Vec<SyncCall> {
        locked(&self.calls).clone()
    }
}

impl ModelSync for RecordingSync {
    fn sync(&self, request: &SyncRequest<'_>) -> Result<ModelSyncResult, AgentRunError> {
        locked(&self.calls).push(SyncCall {
            binary: request.binary.to_owned(),
            base_env: request.base_env.clone(),
            base_url: request.base_url.to_owned(),
            skip_verify: request.skip_verify,
        });
        self.result.as_ref().map_or_else(
            || Err(AgentRunError::NothingToRun),
            |result| Ok(result.clone()),
        )
    }
}

pub(super) struct RecordingPreparer {
    pub(super) args: Vec<String>,
    pub(super) fails: bool,
    pub(super) calls: Mutex<Vec<String>>,
}

impl RecordingPreparer {
    pub(super) fn calls(&self) -> Vec<String> {
        locked(&self.calls).clone()
    }
}

impl Preparer for RecordingPreparer {
    fn prepare(
        &self,
        request: &PrepareRequest<'_>,
        messages: &mut dyn Messages,
    ) -> Result<Vec<String>, AgentRunError> {
        if !self.args.is_empty() {
            messages
                .notice(&format!("prepared {}", self.args.join(" ")))
                .unwrap_or_default();
        }
        locked(&self.calls).push(request.base_url.to_owned());
        if self.fails {
            return Err(AgentRunError::NothingToRun);
        }
        Ok(self.args.clone())
    }
}

pub(super) enum PreparerScript {
    Returning(Vec<String>),
    Failing,
}

pub(super) enum TerminalScript {
    Absent,
    Working,
    Failing(io::ErrorKind),
}

pub(super) struct Observations {
    pub(super) resolver_asked: Vec<String>,
    pub(super) probe_calls: Vec<ProbeCall>,
    pub(super) sync_calls: Vec<SyncCall>,
    pub(super) preparer_calls: Vec<String>,
    pub(super) launches: Vec<LaunchCall>,
    pub(super) spawns: Vec<(WindowsCommand, BTreeMap<String, String>)>,
    pub(super) terminal_calls: usize,
    pub(super) terminal_calls_at_launch: Option<usize>,
    pub(super) warns: Vec<String>,
    pub(super) notices: Vec<String>,
}

pub(super) struct Harness {
    pub(super) command: Vec<String>,
    pub(super) binary: Option<String>,
    pub(super) base_env: EnvVars,
    pub(super) home: Option<PathBuf>,
    pub(super) skip_verify: bool,
    pub(super) probe_status: u16,
    pub(super) probe_failure: Option<io::ErrorKind>,
    pub(super) sync_result: Option<ModelSyncResult>,
    pub(super) preparer: PreparerScript,
    pub(super) launcher_code: i32,
    pub(super) launcher_failure: Option<io::ErrorKind>,
    pub(super) terminal: TerminalScript,
    pub(super) platform: Platform,
    pub(super) messages: RecordingMessages,
}

impl Harness {
    pub(super) fn codex() -> Self {
        Self {
            command: command(&["codex", "--full-auto"]),
            binary: Some(CODEX_BINARY.to_owned()),
            base_env: vars(&[("KEEP", "1")]),
            home: None,
            skip_verify: false,
            probe_status: 200,
            probe_failure: None,
            sync_result: Some(ModelSyncResult::Env(BTreeMap::new())),
            preparer: PreparerScript::Returning(Vec::new()),
            launcher_code: 0,
            launcher_failure: None,
            terminal: TerminalScript::Absent,
            platform: Platform::Unix,
            messages: RecordingMessages::default(),
        }
    }

    pub(super) fn run(&mut self) -> (Result<i32, AgentRunError>, Observations) {
        let fakes = Fakes::build(self);
        let preparers = Preparers {
            pi: &fakes.preparer,
            codex: &fakes.preparer,
        };
        let command = self.command.clone();
        let base_env = self.base_env.clone();
        let api_key = key();
        let request = RunAgentRequest {
            base_url: BASE_URL,
            api_key: &api_key,
            command: &command,
            skip_verify: self.skip_verify,
            base_env: &base_env,
            home: self.home.as_deref(),
            resolver: &fakes.resolver,
            probe: &fakes.probe,
            sync: &fakes.sync,
            preparers: &preparers,
            launcher: &fakes.launcher,
            platform: self.platform,
            terminal: fakes.terminal_ref(),
            messages: &mut self.messages,
        };
        let result = run_agent(request);
        (result, fakes.observe(&self.messages))
    }
}

struct Fakes {
    resolver: RecordingResolver,
    probe: ScriptedProbe,
    sync: RecordingSync,
    preparer: RecordingPreparer,
    launcher: RecordingLauncher,
    terminal: Arc<RecordingTerminal>,
    terminal_attached: bool,
}

impl Fakes {
    fn build(harness: &Harness) -> Self {
        let terminal = Arc::new(build_terminal(harness));
        Self {
            resolver: RecordingResolver::returning(harness.binary.as_deref()),
            probe: build_probe(harness),
            sync: build_sync(harness),
            preparer: build_preparer(harness),
            launcher: build_launcher(harness, &terminal),
            terminal,
            terminal_attached: !matches!(harness.terminal, TerminalScript::Absent),
        }
    }

    fn terminal_ref(&self) -> Option<&dyn Terminal> {
        self.terminal_attached
            .then(|| coerce_terminal(&self.terminal))
    }

    fn observe(&self, messages: &RecordingMessages) -> Observations {
        Observations {
            resolver_asked: self.resolver.asked(),
            probe_calls: self.probe.calls(),
            sync_calls: self.sync.calls(),
            preparer_calls: self.preparer.calls(),
            launches: self.launcher.replacements(),
            spawns: self.launcher.spawns(),
            terminal_calls: self.terminal.call_count(),
            terminal_calls_at_launch: *locked(&self.launcher.terminal_calls_at_launch),
            warns: messages.warns(),
            notices: messages.notices(),
        }
    }
}

fn coerce_terminal(terminal: &RecordingTerminal) -> &dyn Terminal {
    terminal
}

fn build_probe(harness: &Harness) -> ScriptedProbe {
    harness.probe_failure.map_or_else(
        || ScriptedProbe::returning(harness.probe_status),
        ScriptedProbe::failing,
    )
}

fn build_sync(harness: &Harness) -> RecordingSync {
    harness
        .sync_result
        .clone()
        .map_or_else(RecordingSync::failing, RecordingSync::new)
}

fn build_preparer(harness: &Harness) -> RecordingPreparer {
    match &harness.preparer {
        PreparerScript::Returning(args) => RecordingPreparer {
            args: args.clone(),
            fails: false,
            calls: Mutex::new(Vec::new()),
        },
        PreparerScript::Failing => RecordingPreparer {
            args: Vec::new(),
            fails: true,
            calls: Mutex::new(Vec::new()),
        },
    }
}

fn build_terminal(harness: &Harness) -> RecordingTerminal {
    match &harness.terminal {
        TerminalScript::Failing(kind) => RecordingTerminal::failing(*kind),
        TerminalScript::Absent | TerminalScript::Working => RecordingTerminal::working(),
    }
}

fn build_launcher(harness: &Harness, terminal: &Arc<RecordingTerminal>) -> RecordingLauncher {
    let mut launcher = RecordingLauncher::exiting(harness.launcher_code);
    launcher.terminal = Some(Arc::clone(terminal));
    launcher.result = harness
        .launcher_failure
        .map_or(Ok(harness.launcher_code), Err);
    launcher
}

pub(super) fn models_body(rows: &str) -> Vec<u8> {
    format!("{{\"data\":{rows}}}").into_bytes()
}

pub(super) fn limits_body(rows: &str) -> Vec<u8> {
    format!("{{\"data\":{rows}}}").into_bytes()
}

pub(super) fn stock_row(slug: &str, priority: i64) -> String {
    let name = slug.to_owned();
    let rank = priority.to_string();
    format!(
        "{{\"slug\":\"{name}\",\"priority\":{rank},\"visibility\":\"list\",\"supported_in_api\":true,\"context_window\":1000000,\"base_instructions\":\"stock\"}}"
    )
}

pub(super) fn stock_catalog(rows: &[String]) -> String {
    format!("{{\"models\":[{}]}}", rows.join(","))
}

pub(super) fn temp_dir() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

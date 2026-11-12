use std::cell::Cell;
use std::collections::VecDeque;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use betterlite_logging::private_json::files::{FileAccess, LocalFiles, PrivateFiles, StagedTemp};
use secrecy::SecretString;
use tempfile::TempDir;

use crate::autoroute::commands::AutoroutePorts;
use crate::autoroute::commands::args::AutorouteCommand;
use crate::autoroute::commands::entropy::{Entropy, RandomError};
use crate::autoroute::commands::error::CommandsError;
use crate::autoroute::commands::run as dispatch_autoroute;
use crate::autoroute::commands::signals::{StopGuard, StopSignals};
use crate::autoroute::commands::start::start as run_start_command;
use crate::autoroute::commands::stop::stop as run_stop_command;
use crate::autoroute::config::model::RawModelListing;
use crate::autoroute::paths::{config_path, pid_record_path};
use crate::autoroute::process::control::{Clock, SignalOutcome, SignalPort, TermSignal};
use crate::autoroute::process::error::ProcessError;
use crate::autoroute::process::launcher::{
    ExitCode, LaunchRequest, LaunchedProcess, ProcessLauncher,
};
use crate::autoroute::process::liveliness::LivelinessProbe;
use crate::autoroute::process::log_tail::{LogTail, TailWorker};
use crate::autoroute::process::pid_record::{PidRecord, read_pid_record};
use crate::autoroute::process::port::PortProbe;
use crate::autoroute::process::runtime::GatewayRuntime;
use crate::autoroute::wizard::error::{ModelDiscoveryError, WizardError};
use crate::autoroute::wizard::{ModelDiscoveryClient, Prompter};
use crate::claude_settings::SettingsDocument;
use crate::claude_settings::paths::claude_settings_paths;
use crate::cli_context::CliContext;
use crate::settings_backup::read::read_backup;
use crate::settings_backup::record::BackupRecord;

pub(crate) const LAUNCHED_PID: u32 = 4242;
pub(crate) const PERSISTED_KEY: &str = "sk-persisted-master-key";
pub(crate) const CLI_BASE_URL: &str = "http://127.0.0.1:4000";
pub(crate) const CLI_API_KEY: &str = "sk-cli-key";

pub(crate) struct FakeGateway {
    pub executable: Option<PathBuf>,
    pub runs: bool,
}

impl Default for FakeGateway {
    fn default() -> Self {
        Self {
            executable: Some(PathBuf::from("/usr/local/bin/betterlite")),
            runs: true,
        }
    }
}

impl GatewayRuntime for FakeGateway {
    fn find(&self) -> Option<PathBuf> {
        self.executable.clone()
    }

    fn runs(&self, _executable: &Path) -> bool {
        self.runs
    }
}

pub(crate) struct FakePortProbe {
    pub available: bool,
    pub probed: Mutex<Vec<u16>>,
}

impl Default for FakePortProbe {
    fn default() -> Self {
        Self {
            available: true,
            probed: Mutex::new(Vec::new()),
        }
    }
}

impl PortProbe for FakePortProbe {
    fn is_available(&self, port: u16) -> bool {
        self.probed.lock().unwrap().push(port);
        self.available
    }
}

#[derive(Default)]
pub(crate) struct RecordingTail {
    pub starts: Mutex<Vec<PathBuf>>,
}

impl RecordingTail {
    pub(crate) fn started(&self) -> Vec<PathBuf> {
        self.starts.lock().unwrap().clone()
    }
}

impl LogTail for RecordingTail {
    fn start(&self, log_path: &Path) -> Result<Box<dyn TailWorker>, ProcessError> {
        if let Ok(mut starts) = self.starts.lock() {
            starts.push(log_path.to_owned());
        }
        Ok(Box::new(QuietTailWorker))
    }
}

struct QuietTailWorker;

impl TailWorker for QuietTailWorker {}

pub(crate) struct FakeLiveliness {
    pub statuses: Mutex<Vec<bool>>,
    pub answer_when_empty: bool,
    pub urls: Mutex<Vec<String>>,
}

impl Default for FakeLiveliness {
    fn default() -> Self {
        Self {
            statuses: Mutex::new(Vec::new()),
            answer_when_empty: true,
            urls: Mutex::new(Vec::new()),
        }
    }
}

impl LivelinessProbe for FakeLiveliness {
    fn is_live(&self, url: &str) -> bool {
        self.urls.lock().unwrap().push(url.to_owned());
        let mut statuses = self.statuses.lock().unwrap();
        if statuses.is_empty() {
            return self.answer_when_empty;
        }
        statuses.remove(0)
    }
}

pub(crate) struct FakeSignals {
    pub alive: Mutex<Vec<u32>>,
    pub sent: Mutex<Vec<(u32, TermSignal)>>,
}

impl Default for FakeSignals {
    fn default() -> Self {
        Self {
            alive: Mutex::new(vec![LAUNCHED_PID]),
            sent: Mutex::new(Vec::new()),
        }
    }
}

impl FakeSignals {
    pub(crate) fn sent(&self) -> Vec<(u32, TermSignal)> {
        self.sent.lock().unwrap().clone()
    }
}

impl SignalPort for FakeSignals {
    fn probe(&self, pid: u32) -> SignalOutcome {
        if self.alive.lock().unwrap().contains(&pid) {
            SignalOutcome::Exists
        } else {
            SignalOutcome::Missing
        }
    }

    fn send(&self, pid: u32, signal: TermSignal) {
        self.sent.lock().unwrap().push((pid, signal));
        self.alive.lock().unwrap().retain(|alive| *alive != pid);
    }
}

pub(crate) struct FakeClock {
    base: Instant,
    elapsed: Cell<Duration>,
}

impl Default for FakeClock {
    fn default() -> Self {
        Self {
            base: Instant::now(),
            elapsed: Cell::new(Duration::ZERO),
        }
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Instant {
        self.base
            .checked_add(self.elapsed.get())
            .unwrap_or(self.base)
    }

    fn sleep(&self, duration: Duration) {
        let next = self
            .elapsed
            .get()
            .checked_add(duration)
            .unwrap_or_else(|| self.elapsed.get());
        self.elapsed.set(next);
    }
}

#[derive(Default)]
pub(crate) struct FixedEntropy {
    pub pattern: Vec<u8>,
    pub filled: usize,
}

impl Entropy for FixedEntropy {
    fn fill(&mut self, bytes: &mut [u8]) -> Result<(), RandomError> {
        self.filled = bytes.len();
        for (index, slot) in bytes.iter_mut().enumerate() {
            *slot = self.pattern.get(index).copied().unwrap_or(0);
        }
        Ok(())
    }
}

#[derive(Default)]
pub(crate) struct FakeClient {
    pub models: Vec<RawModelListing>,
}

impl ModelDiscoveryClient for FakeClient {
    fn list_models(&self) -> Result<Vec<RawModelListing>, ModelDiscoveryError> {
        Ok(self.models.clone())
    }
}

#[derive(Default)]
pub(crate) struct ScriptedPrompter {
    pub selections: VecDeque<Vec<String>>,
    pub confirm_answers: VecDeque<bool>,
    pub announcements: Vec<String>,
}

impl Prompter for ScriptedPrompter {
    fn announce(&mut self, message: &str) -> Result<(), WizardError> {
        self.announcements.push(message.to_owned());
        Ok(())
    }

    fn select_one(&mut self, _message: &str, _choices: &[String]) -> Result<String, WizardError> {
        Ok(String::new())
    }

    fn select_many(
        &mut self,
        _message: &str,
        _choices: &[String],
    ) -> Result<Vec<String>, WizardError> {
        Ok(self.selections.pop_front().unwrap_or_default())
    }

    fn confirm(&mut self, _message: &str, _default: bool) -> Result<bool, WizardError> {
        Ok(self.confirm_answers.pop_front().unwrap_or(false))
    }

    fn prompt_keywords(&mut self, _message: &str, default: &str) -> Result<String, WizardError> {
        Ok(default.to_owned())
    }
}

#[derive(Clone)]
pub(crate) struct RecordedLaunch {
    pub executable: PathBuf,
    pub arguments: Vec<String>,
    pub log_path: PathBuf,
}

pub(crate) struct FakeLauncher {
    pub pid: u32,
    pub exit: Option<ExitCode>,
    pub requests: Mutex<Vec<RecordedLaunch>>,
}

impl Default for FakeLauncher {
    fn default() -> Self {
        Self {
            pid: LAUNCHED_PID,
            exit: None,
            requests: Mutex::new(Vec::new()),
        }
    }
}

impl ProcessLauncher for FakeLauncher {
    fn launch(
        &self,
        request: &LaunchRequest<'_>,
    ) -> Result<Box<dyn LaunchedProcess>, ProcessError> {
        if let Ok(mut requests) = self.requests.lock() {
            requests.push(RecordedLaunch {
                executable: request.executable.to_owned(),
                arguments: request.arguments.to_vec(),
                log_path: request.log_path.to_owned(),
            });
        }
        Ok(Box::new(FakeProcess {
            pid: self.pid,
            exit: self.exit,
        }))
    }
}

struct FakeProcess {
    pid: u32,
    exit: Option<ExitCode>,
}

impl LaunchedProcess for FakeProcess {
    fn pid(&self) -> u32 {
        self.pid
    }

    fn has_exited(&mut self) -> Result<Option<ExitCode>, ProcessError> {
        Ok(self.exit)
    }
}

#[derive(Clone, Default)]
pub(crate) struct StopSnapshot {
    pub record: Option<PidRecord>,
    pub settings: Option<SettingsDocument>,
    pub backup: Option<BackupRecord>,
}

pub(crate) struct SnapshotStopSignals {
    pub pid_record: PathBuf,
    pub settings: PathBuf,
    pub backup: PathBuf,
    pub snapshot: Arc<Mutex<Option<StopSnapshot>>>,
}

impl StopSignals for SnapshotStopSignals {
    fn watch(&self) -> Result<Box<dyn StopGuard>, CommandsError> {
        Ok(Box::new(SnapshotAtStop {
            pid_record: self.pid_record.clone(),
            settings: self.settings.clone(),
            backup: self.backup.clone(),
            snapshot: Arc::clone(&self.snapshot),
        }))
    }
}

struct SnapshotAtStop {
    pid_record: PathBuf,
    settings: PathBuf,
    backup: PathBuf,
    snapshot: Arc<Mutex<Option<StopSnapshot>>>,
}

impl StopGuard for SnapshotAtStop {
    fn wait(&self) {
        let settings = fs::read_to_string(&self.settings)
            .ok()
            .and_then(|text| serde_json::from_str::<SettingsDocument>(&text).ok());
        let backup = read_backup(&self.backup).ok().flatten();
        let record = read_pid_record(&self.pid_record).ok().flatten();
        if let Ok(mut snapshot) = self.snapshot.lock() {
            *snapshot = Some(StopSnapshot {
                record,
                settings,
                backup,
            });
        }
    }
}

pub(crate) struct BackupOrderFiles {
    pub inner: LocalFiles,
    pub backup_path: PathBuf,
    pub backup_seen: Arc<Mutex<Vec<bool>>>,
}

impl FileAccess for BackupOrderFiles {
    fn create_parents(&self, directory: &Path) -> io::Result<()> {
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
        if let Ok(mut seen) = self.backup_seen.lock() {
            seen.push(self.backup_path.exists());
        }
        self.inner.create_staged(directory, suffix)
    }

    fn write_staged(&self, staged: &mut StagedTemp, bytes: &[u8]) -> io::Result<()> {
        self.inner.write_staged(staged, bytes)
    }

    fn replace(&self, staged: &Path, target: &Path) -> io::Result<()> {
        self.inner.replace(staged, target)
    }

    fn remove(&self, path: &Path) -> io::Result<()> {
        self.inner.remove(path)
    }

    fn rewrite_existing(&self, target: &Path, bytes: &[u8]) -> io::Result<()> {
        self.inner.rewrite_existing(target, bytes)
    }
}

pub(crate) struct Fixture {
    pub home: TempDir,
    pub context: CliContext,
    pub client: FakeClient,
    pub prompter: ScriptedPrompter,
    pub stdin_is_terminal: bool,
    pub gateway: FakeGateway,
    pub launcher: FakeLauncher,
    pub port_probe: FakePortProbe,
    pub liveliness: FakeLiveliness,
    pub tail: RecordingTail,
    pub stop_signals: SnapshotStopSignals,
    pub signals: FakeSignals,
    pub clock: FakeClock,
    pub files: PrivateFiles,
    pub entropy: FixedEntropy,
    pub binary_path: PathBuf,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub snapshot: Arc<Mutex<Option<StopSnapshot>>>,
}

const DEFAULT_CONFIG: &str =
    "model_list: []\ngeneral_settings:\n  master_key: sk-persisted-master-key\n";

pub(crate) fn default_fixture() -> Fixture {
    let home = TempDir::new().unwrap();
    let paths = claude_settings_paths(home.path());
    fs::create_dir_all(paths.autoroute_backup.parent().unwrap()).unwrap();
    fs::write(config_path(home.path()), DEFAULT_CONFIG).unwrap();
    let snapshot = Arc::new(Mutex::new(None));
    let stop_signals = SnapshotStopSignals {
        pid_record: pid_record_path(home.path()),
        settings: paths.settings,
        backup: paths.autoroute_backup,
        snapshot: Arc::clone(&snapshot),
    };
    Fixture {
        context: CliContext {
            base_url: CLI_BASE_URL.to_owned(),
            base_url_explicit: true,
            api_key: Some(SecretString::from(CLI_API_KEY)),
            api_key_from_token_file: false,
        },
        client: FakeClient::default(),
        prompter: ScriptedPrompter::default(),
        stdin_is_terminal: true,
        gateway: FakeGateway::default(),
        launcher: FakeLauncher::default(),
        port_probe: FakePortProbe::default(),
        liveliness: FakeLiveliness::default(),
        tail: RecordingTail::default(),
        stop_signals,
        signals: FakeSignals::default(),
        clock: FakeClock::default(),
        files: PrivateFiles::default(),
        entropy: FixedEntropy::default(),
        binary_path: PathBuf::from("/usr/local/bin/lite"),
        stdout: Vec::new(),
        stderr: Vec::new(),
        snapshot,
        home,
    }
}

impl Fixture {
    pub(crate) fn ports(&mut self) -> AutoroutePorts<'_> {
        AutoroutePorts {
            context: &self.context,
            home: self.home.path(),
            client: &self.client,
            prompter: &mut self.prompter,
            stdin_is_terminal: self.stdin_is_terminal,
            gateway: &self.gateway,
            launcher: &self.launcher,
            port_probe: &self.port_probe,
            liveliness: &self.liveliness,
            signals: &self.stop_signals,
            signal_port: &self.signals,
            tail: &self.tail,
            clock: &self.clock,
            files: &self.files,
            entropy: &mut self.entropy,
            binary_path: Some(&self.binary_path),
            stdout: &mut self.stdout,
            stderr: &mut self.stderr,
        }
    }

    pub(crate) fn run(&mut self, command: AutorouteCommand) -> Result<(), CommandsError> {
        let mut ports = self.ports();
        dispatch_autoroute(command, &mut ports)
    }

    pub(crate) fn start(&mut self, port: u16) -> Result<(), CommandsError> {
        let mut ports = self.ports();
        run_start_command(&mut ports, port)
    }

    pub(crate) fn stop(&mut self) -> Result<(), CommandsError> {
        let mut ports = self.ports();
        run_stop_command(&mut ports)
    }

    pub(crate) fn write_config(&self, yaml: &str) {
        fs::write(config_path(self.home.path()), yaml).unwrap();
    }

    pub(crate) fn config_path(&self) -> PathBuf {
        config_path(self.home.path())
    }

    pub(crate) fn pid_record_path(&self) -> PathBuf {
        pid_record_path(self.home.path())
    }

    pub(crate) fn settings_path(&self) -> PathBuf {
        claude_settings_paths(self.home.path()).settings
    }

    pub(crate) fn backup_path(&self) -> PathBuf {
        claude_settings_paths(self.home.path()).autoroute_backup
    }

    pub(crate) fn snapshot(&self) -> StopSnapshot {
        self.snapshot.lock().unwrap().clone().unwrap_or_default()
    }

    pub(crate) fn stdout_text(&self) -> String {
        String::from_utf8(self.stdout.clone()).unwrap()
    }

    pub(crate) fn stderr_text(&self) -> String {
        String::from_utf8(self.stderr.clone()).unwrap()
    }
}

use std::io;
use std::path::PathBuf;

use thiserror::Error;

use super::entropy::RandomError;
use crate::autoroute::process::error::ProcessError;
use crate::autoroute::wizard::error::WizardError;
use crate::claude_settings::error::ClaudeSettingsError;
use crate::settings_backup::error::BackupError;

#[derive(Debug, Error)]
pub enum CommandsError {
    #[error("No config found. Run `lite autoroute configure` first.")]
    MissingConfig,
    #[error("An ephemeral proxy is already running (pid {pid}). Run `lite autoroute stop` first.")]
    AlreadyRunning { pid: u32 },
    #[error(
        "{path} already exists -- `lite autoroute start` looks like it's already running (or crashed without cleanup). Run `lite autoroute stop` first."
    )]
    BackupExists { path: PathBuf },
    #[error(
        "Port 4000 is the proxy's own default and its launcher silently rebinds it to a random port when busy; pick a different --port."
    )]
    ReservedPort,
    #[error(
        "Port {port} on 127.0.0.1 is already in use. If a previous `lite autoroute start` is still running or crashed, run `lite autoroute stop`; otherwise pick a different port with --port."
    )]
    PortInUse { port: u16 },
    #[error("the started gateway reported no process id")]
    MissingPid,
    #[error("cannot read {path}: {source}")]
    ReadConfig {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("{path} is empty or corrupt. Run `lite autoroute configure` again to regenerate it.")]
    CorruptConfig {
        path: PathBuf,
        #[source]
        source: Box<serde_saphyr::Error>,
    },
    #[error("{path} is empty or corrupt. Run `lite autoroute configure` again to regenerate it.")]
    EmptyConfig { path: PathBuf },
    #[error("cannot encode the generated config {path}: {source}")]
    EncodeConfig {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("cannot serialize the generated config {path}: {source}")]
    SerializeConfig {
        path: PathBuf,
        #[source]
        source: Box<serde_saphyr::SerializeError>,
    },
    #[error("cannot write {path}: {source}")]
    WriteConfig {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("cannot create the directory for {path}: {source}")]
    SettingsDirectory {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("{path} is not valid UTF-8, so it cannot be passed to the gateway")]
    NonUtf8Path { path: PathBuf },
    #[error("no API key is configured; run `lite login` or pass --api-key")]
    MissingApiKey,
    #[error("failed to write to the terminal: {source}")]
    Output {
        #[source]
        source: io::Error,
    },
    #[error("the file write runtime could not start: {source}")]
    Runtime {
        #[source]
        source: io::Error,
    },
    #[error("cannot watch for SIGINT and SIGTERM: {source}")]
    Signals {
        #[source]
        source: io::Error,
    },
    #[error("this platform cannot watch for SIGINT and SIGTERM")]
    SignalsUnsupported,
    #[error(transparent)]
    Process(#[from] ProcessError),
    #[error(transparent)]
    Wizard(#[from] WizardError),
    #[error(transparent)]
    Backup(#[from] BackupError),
    #[error(transparent)]
    Settings(#[from] ClaudeSettingsError),
    #[error(transparent)]
    Random(#[from] RandomError),
}

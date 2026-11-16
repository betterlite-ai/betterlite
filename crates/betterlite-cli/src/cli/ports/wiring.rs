use std::ffi::OsString;
use std::io::Write as _;
use std::path::PathBuf;

use betterlite_logging::private_json::files::PrivateFiles;

use crate::agent_launch::{StdLauncher, StdTerminal as AgentTerminal};
use crate::agent_run::codex::StdDebugModels;
use crate::agent_run::prepare::StdStatuslineInstaller;
use crate::agent_run::resolver::PathResolver;
use crate::credential_lifecycle::files::ManagedTokenFile;
use crate::login::NeverInterrupted;
use crate::sso_polling::polling::ThreadSleeper;
use crate::sso_polling::terminal::Terminal as SelectionTerminal;

use super::super::Failure;
use super::super::session::{Session, SystemEntropy};
use super::super::streams::Channel;

pub(crate) fn token_path(session: &Session<'_>) -> Result<PathBuf, Failure> {
    session
        .token_path
        .clone()
        .ok_or_else(|| Failure::Plain(String::from("cannot find your home directory")))
}

pub(crate) fn token_file(session: &Session<'_>) -> Result<ManagedTokenFile, Failure> {
    Ok(ManagedTokenFile {
        files: PrivateFiles::default(),
        path: token_path(session)?,
    })
}

pub(crate) fn message_sink(channel: Channel) -> impl Fn(&str) + Send + Sync {
    move |message: &str| {
        let mut sink = channel.clone();
        let _unused = writeln!(sink, "{message}");
    }
}

pub(crate) fn no_browser(_url: &str) {}

pub(crate) fn path_resolver(session: &Session<'_>) -> PathResolver {
    PathResolver {
        search_path: OsString::from(session.environment.lookup("PATH").unwrap_or_default()),
        current_dir: std::env::current_dir().unwrap_or_else(|_error| PathBuf::from(".")),
    }
}

pub(crate) struct Wiring {
    pub files: ManagedTokenFile,
    pub private_files: PrivateFiles,
    pub token_path: PathBuf,
    pub selection: SelectionTerminal,
    pub entropy: SystemEntropy,
    pub terminal: AgentTerminal,
    pub resolver: PathResolver,
    pub installer: StdStatuslineInstaller,
    pub debug: StdDebugModels,
    pub sleeper: ThreadSleeper,
    pub launcher: StdLauncher,
    pub interrupt: NeverInterrupted,
}

impl Wiring {
    pub(crate) fn new(session: &Session<'_>) -> Result<Self, Failure> {
        let token_path = token_path(session)?;
        Ok(Self {
            files: ManagedTokenFile {
                files: PrivateFiles::default(),
                path: token_path.clone(),
            },
            private_files: PrivateFiles::default(),
            token_path,
            selection: SelectionTerminal::default(),
            entropy: SystemEntropy,
            terminal: AgentTerminal,
            resolver: path_resolver(session),
            installer: StdStatuslineInstaller,
            debug: StdDebugModels,
            sleeper: ThreadSleeper,
            launcher: StdLauncher,
            interrupt: NeverInterrupted,
        })
    }
}

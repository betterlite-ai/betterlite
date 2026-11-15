use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, PoisonError};

use betterlite_cli::cli_context::CliContext;
use betterlite_cli::login::NeverInterrupted;
use betterlite_cli::sso_polling::terminal::{SelectionInput, Terminal as SelectionTerminal};
use betterlite_cli::up::ports::UpPorts;
use betterlite_logging::private_json::files::PrivateFiles;
use secrecy::SecretString;
use serde_json::{Value, json};
use tempfile::TempDir;

use crate::helpers::HelperError;

use super::Input;
use super::agents::{FakeEnvironment, FakeTerminal, RecordingProbe, ScriptedLogin, ScriptedStop};
use super::fakes::{
    DisabledVault, FixedClock, NoSleeper, ScriptedFiles, State, UnusedHttp, ZeroEntropy, json_file,
};

pub(crate) const BASE_URL: &str = "http://gateway.test:4000";

const fn ignore_message(_message: &str) {}

pub(crate) struct World {
    pub(crate) context: CliContext,
    pub(crate) environment: FakeEnvironment,
    pub(crate) files: ScriptedFiles,
    pub(crate) vault: DisabledVault,
    pub(crate) private_files: PrivateFiles,
    pub(crate) http: Arc<UnusedHttp>,
    pub(crate) clock: Arc<FixedClock>,
    pub(crate) entropy: ZeroEntropy,
    pub(crate) sleeper: NoSleeper,
    pub(crate) selection: SelectionTerminal,
    pub(crate) interrupt: NeverInterrupted,
    pub(crate) token_path: PathBuf,
    pub(crate) home: TempDir,
    pub(crate) probe: RecordingProbe,
    pub(crate) terminal: FakeTerminal,
    pub(crate) state: Arc<State>,
    pub(crate) stop_kind: Option<super::StopKind>,
}

impl World {
    pub(crate) fn new(input: &Input) -> Result<Self, Box<HelperError>> {
        let home = tempfile::tempdir().map_err(|source| helper_failure(&source))?;
        let state = Arc::new(State::default());
        seed_files(input, home.path(), &state)?;
        Ok(Self {
            context: CliContext {
                base_url: BASE_URL.to_owned(),
                base_url_explicit: false,
                api_key: input
                    .api_key
                    .as_ref()
                    .map(|key| SecretString::from(key.clone())),
                api_key_from_token_file: input.api_key_from_token_file,
            },
            environment: FakeEnvironment {
                home: home.path().to_path_buf(),
            },
            files: ScriptedFiles(Arc::clone(&state)),
            vault: DisabledVault,
            private_files: PrivateFiles::default(),
            http: Arc::new(UnusedHttp),
            clock: Arc::new(FixedClock),
            entropy: ZeroEntropy,
            sleeper: NoSleeper,
            selection: SelectionTerminal::default(),
            interrupt: NeverInterrupted,
            token_path: home.path().join(".litellm").join("token.json"),
            home,
            probe: RecordingProbe {
                status: input.verify_status.unwrap_or(200),
                state: Arc::clone(&state),
            },
            terminal: FakeTerminal {
                interactive: input.tty,
            },
            state,
            stop_kind: input.stop,
        })
    }

    pub(crate) fn scripted_login(&self, input: &Input) -> ScriptedLogin {
        ScriptedLogin {
            token: input.login_token.clone(),
            state: Arc::clone(&self.state),
        }
    }

    pub(crate) fn scripted_stop(&self) -> ScriptedStop {
        ScriptedStop {
            state: Arc::clone(&self.state),
            settings: self.home.path().join(".claude").join("settings.json"),
            backup: self
                .home
                .path()
                .join(".litellm")
                .join("claude_settings_backup.json"),
        }
    }

    pub(crate) fn bind<'world>(
        &'world mut self,
        login: &'world mut ScriptedLogin,
        stop: &'world ScriptedStop,
        stderr: &'world mut Vec<u8>,
        stdout: &'world mut Vec<u8>,
    ) -> UpPorts<'world, UnusedHttp, ZeroEntropy, FixedClock> {
        let selection: &'world mut dyn SelectionInput = &mut self.selection;
        UpPorts {
            context: &self.context,
            environment: &self.environment,
            files: &self.files,
            vault: &mut self.vault,
            private_files: &self.private_files,
            http: Arc::clone(&self.http),
            clock: Arc::clone(&self.clock),
            entropy: &mut self.entropy,
            sleeper: &self.sleeper,
            selection,
            browser: &ignore_message,
            interrupt: &self.interrupt,
            token_path: &self.token_path,
            binary_path: Some(Path::new("/opt/betterlite")),
            home: Some(self.home.path()),
            expiration_hours: 24,
            warn: &ignore_message,
            terminal: Some(&self.terminal),
            login,
            probe: &self.probe,
            signals: stop,
            stdout,
            stderr,
        }
    }

    pub(crate) fn answer(&self, stdout: &[u8], error: Option<&Value>) -> Value {
        let root = self.home.path().display().to_string();
        let text = String::from_utf8_lossy(stdout).replace(&root, "<tmp>");
        json!({
            "stdout": text,
            "error": error,
            "login_calls": self.state.logins.load(Ordering::SeqCst),
            "probe": self.state.probe.lock().unwrap_or_else(PoisonError::into_inner).clone(),
            "stopped_by": self.stop_kind,
            "settings": json_file(&self.home.path().join(".claude").join("settings.json")),
            "backup": json_file(&self.home.path().join(".litellm").join("claude_settings_backup.json")),
            "token": self.state.record.lock().unwrap_or_else(PoisonError::into_inner).clone(),
            "snapshot": self.state.snapshot.lock().unwrap_or_else(PoisonError::into_inner).clone(),
        })
    }
}

fn seed_files(input: &Input, root: &Path, state: &State) -> Result<(), Box<HelperError>> {
    if let Some(record) = &input.stored {
        let bytes = serde_json::to_vec(record).map_err(|source| helper_failure(&source))?;
        *state.record.lock().unwrap_or_else(PoisonError::into_inner) = Some(record.clone());
        write_seed(&root.join(".litellm").join("token.json"), &bytes)?;
    }
    if let Some(settings) = &input.settings {
        write_seed(
            &root.join(".claude").join("settings.json"),
            settings.as_bytes(),
        )?;
    }
    if let Some(backup) = &input.backup {
        write_seed(
            &root.join(".litellm").join("claude_settings_backup.json"),
            backup.as_bytes(),
        )?;
    }
    Ok(())
}

fn write_seed(path: &Path, bytes: &[u8]) -> Result<(), Box<HelperError>> {
    if let Some(directory) = path.parent() {
        fs::create_dir_all(directory).map_err(|source| helper_failure(&source))?;
    }
    fs::write(path, bytes).map_err(|source| helper_failure(&source))
}

pub(crate) fn helper_failure<M>(message: &M) -> Box<HelperError>
where
    M: std::fmt::Display,
{
    Box::new(HelperError {
        class: "Fixture".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_string(),
    })
}

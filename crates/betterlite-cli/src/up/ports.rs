use std::io::Write;
use std::path::Path;
use std::sync::Arc;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_logging::private_json::files::PrivateFiles;

use crate::agent_launch::{KeyProbe, Terminal};
use crate::cli_context::CliContext;
use crate::credential_lifecycle::files::CredentialFiles;
use crate::credentials::notice::MessageSink;
use crate::keyring::SecretVault;
use crate::login::error::LoginError;
use crate::login::ports::{FlowPorts, LoginPorts, TerminalWriter};
use crate::login::{Interrupts, LoginArgs, run_login};
use crate::pkce_login::{Clock, Entropy, Http, WallClock};
use crate::sso_polling::polling::Sleeper;
use crate::sso_polling::terminal::SelectionInput;
use crate::sso_polling::transport::PollTransport;

use super::error::UpError;

// LiteLLM's login command takes the SSO clock as both the monotonic and the wall clock
pub type UpFlow<'flow, H, E, W> = FlowPorts<'flow, H, W, E, W>;

pub struct LoginAttempt<'attempt, H, E, W> {
    pub pkce: bool,
    pub flow: UpFlow<'attempt, H, E, W>,
    pub stdout: &'attempt mut TerminalWriter,
}

pub trait LoginLauncher<H, E, W> {
    fn start(&mut self, attempt: LoginAttempt<'_, H, E, W>) -> Result<(), LoginError>;
}

pub struct FlowLogin;

impl<H, E, W> LoginLauncher<H, E, W> for FlowLogin
where
    H: Http + PollTransport + Send + Sync + 'static,
    E: Entropy,
    W: Clock + WallClock + Send + Sync + 'static,
{
    fn start(&mut self, attempt: LoginAttempt<'_, H, E, W>) -> Result<(), LoginError> {
        run_login(
            LoginArgs {
                config_claude: false,
                pkce: attempt.pkce,
            },
            &mut LoginPorts {
                flow: attempt.flow,
                stdout: attempt.stdout,
            },
        )
    }
}

pub trait StopSource {
    fn install(&self) -> Result<(), UpError>;
    fn wait(&self);
}

pub struct UpPorts<'ports, H, E, W> {
    pub context: &'ports CliContext,
    pub environment: &'ports (dyn EnvironmentLookup + Send + Sync),
    pub files: &'ports dyn CredentialFiles,
    pub vault: &'ports mut (dyn SecretVault + Send + Sync),
    pub private_files: &'ports PrivateFiles,
    pub http: Arc<H>,
    pub clock: Arc<W>,
    pub entropy: &'ports mut E,
    pub sleeper: &'ports dyn Sleeper,
    pub selection: &'ports mut dyn SelectionInput,
    pub browser: MessageSink<'ports>,
    pub interrupt: &'ports dyn Interrupts,
    pub token_path: &'ports Path,
    pub binary_path: Option<&'ports Path>,
    pub home: Option<&'ports Path>,
    pub expiration_hours: i64,
    pub warn: MessageSink<'ports>,
    pub terminal: Option<&'ports dyn Terminal>,
    pub login: &'ports mut dyn LoginLauncher<H, E, W>,
    pub probe: &'ports dyn KeyProbe,
    pub signals: &'ports dyn StopSource,
    pub stdout: &'ports mut TerminalWriter,
    pub stderr: &'ports mut dyn Write,
}

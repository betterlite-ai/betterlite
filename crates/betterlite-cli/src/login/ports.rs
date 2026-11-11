use std::io::Write;
use std::path::Path;
use std::sync::Arc;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_logging::private_json::files::PrivateFiles;

use crate::cli_context::CliContext;
use crate::credential_lifecycle::files::CredentialFiles;
use crate::credentials::notice::MessageSink;
use crate::keyring::SecretVault;
use crate::sso_polling::polling::Sleeper;
use crate::sso_polling::terminal::SelectionInput;

use super::Interrupts;

pub type TerminalWriter = dyn Write + Send + 'static;

pub struct LoginPorts<'login, H, C, E, W> {
    pub flow: FlowPorts<'login, H, C, E, W>,
    pub stdout: &'login mut TerminalWriter,
}

pub struct FlowPorts<'login, H, C, E, W> {
    pub context: &'login CliContext,
    pub environment: &'login (dyn EnvironmentLookup + Send + Sync),
    pub files: &'login dyn CredentialFiles,
    pub vault: &'login mut (dyn SecretVault + Send + Sync),
    pub private_files: &'login PrivateFiles,
    pub http: Arc<H>,
    pub clock: &'login C,
    pub wall_clock: &'login W,
    pub entropy: &'login mut E,
    pub sleeper: &'login dyn Sleeper,
    pub selection: &'login mut dyn SelectionInput,
    pub browser: MessageSink<'login>,
    pub interrupt: &'login dyn Interrupts,
    pub token_path: &'login Path,
    pub binary_path: Option<&'login Path>,
}

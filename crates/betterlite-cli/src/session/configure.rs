use std::io;
use std::io::Write;
use std::path::Path;

use secrecy::SecretString;
use tokio::runtime::Builder;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_logging::private_json::files::PrivateFiles;

use crate::claude_settings::choices::{ModelChoice, STARTING_MODEL_ROLE, StaticToken};
use crate::claude_settings::paths::{
    claude_settings_path, claude_settings_paths, configure_state_path, settings_file_owners,
};
use crate::claude_settings::receipts::configure::{ConfigureRequest, configure_claude_settings};
use crate::claude_settings::receipts::error::ReceiptsError;

use super::error::SessionError;

pub struct ConfigureClaudeRequest<'request> {
    pub base_url: &'request str,
    pub api_key: &'request SecretString,
    pub environment: &'request dyn EnvironmentLookup,
    pub files: &'request PrivateFiles,
    pub binary_path: Option<&'request Path>,
    pub stdout: &'request mut dyn Write,
}

pub fn configure_claude_code(request: &mut ConfigureClaudeRequest<'_>) -> Result<(), SessionError> {
    let settings = claude_settings_path(request.environment).ok_or(SessionError::NoHome)?;
    let home = request
        .environment
        .system_home()
        .ok_or(SessionError::NoHome)?;
    let paths = claude_settings_paths(&home);
    let state = configure_state_path(&settings, &paths).map_err(ReceiptsError::from)?;
    let owners = settings_file_owners(&settings, &paths).map_err(ReceiptsError::from)?;
    let token = StaticToken {
        token: request.api_key.clone(),
    };
    let model = ModelChoice::KeepModel;
    let configure = ConfigureRequest {
        base_url: request.base_url,
        credential: &token,
        model: &model,
        settings_path: &settings,
        state_path: &state,
        owners: &owners,
        binary_path: request.binary_path,
    };
    let runtime = Builder::new_current_thread()
        .build()
        .map_err(|source| SessionError::Runtime { source })?;
    let configured = runtime.block_on(configure_claude_settings(
        &configure,
        request.files,
        request.files,
    ));
    configured?;
    let _printed = print_configured(&settings, request.base_url, &mut *request.stdout);
    Ok(())
}

fn print_configured(
    settings_path: &Path,
    base_url: &str,
    stdout: &mut dyn Write,
) -> io::Result<()> {
    let settings = settings_path.display();
    let root = base_url.trim_end_matches('/');
    writeln!(
        stdout,
        "\nConfigured Claude Code: {settings} now routes through {root}."
    )?;
    writeln!(
        stdout,
        "This login's key is stored in the file, so run `lite login --config-claude` again after it expires. Your other Claude Code settings were left untouched. Restart Claude Code to pick this up. Undo with `lite unconfigure claude`; `lite configure claude --model` sets {STARTING_MODEL_ROLE}."
    )
}

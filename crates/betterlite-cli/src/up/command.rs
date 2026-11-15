use std::fs;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use betterlite_logging::private_json::files::PrivateFiles;
use secrecy::SecretString;

use crate::agent_launch::{AgentRunError, verify_proxy_key};
use crate::claude_settings::SettingsDocument;
use crate::claude_settings::choices::{ModelChoice, StaticToken};
use crate::claude_settings::io::{load_json_or_empty, write_claude_settings};
use crate::claude_settings::merge::{MergeOptions, merge_claude_settings};
use crate::claude_settings::paths::claude_settings_paths;
use crate::claude_settings::statusline_install::install_statusline_script;
use crate::login::ports::TerminalWriter;
use crate::pkce_login::{Clock, Entropy, Http, WallClock};
use crate::settings_backup::record::BackupRecord;
use crate::settings_backup::write::write_backup;
use crate::sso_polling::transport::PollTransport;

use super::error::UpError;
use super::login::{ensure_fresh_login, resolve_api_key};
use super::ports::UpPorts;
use super::restore::RestoreOnce;
use super::runtime::block_on;

pub fn run_up<H, E, W>(ports: &mut UpPorts<'_, H, E, W>) -> Result<(), UpError>
where
    H: Http + PollTransport + Send + Sync + 'static,
    E: Entropy,
    W: Clock + WallClock + Send + Sync + 'static,
{
    let base_url = ports.context.base_url.clone();
    ensure_fresh_login(ports)?;
    let api_key = resolve_api_key(ports)?;
    verify_key(ports, &base_url, &api_key)?;

    let home = ports.home.ok_or(UpError::NoHome)?;
    patch_settings(
        ports.binary_path,
        ports.private_files,
        home,
        &base_url,
        api_key,
    )?;
    announce(&mut *ports.stdout, &base_url)?;

    ports.signals.install()?;
    ports.signals.wait();
    let mut cleanup = RestoreOnce {
        home,
        files: ports.private_files,
        stdout: &mut *ports.stdout,
        stderr: &mut *ports.stderr,
        done: AtomicBool::new(false),
    };
    cleanup.restore();
    Ok(())
}

fn patch_settings(
    binary_path: Option<&Path>,
    files: &PrivateFiles,
    home: &Path,
    base_url: &str,
    api_key: SecretString,
) -> Result<(), UpError> {
    let paths = claude_settings_paths(home);
    if paths.backup.exists() {
        return Err(UpError::BackupExists { path: paths.backup });
    }
    let status_line = install_statusline_script(binary_path)?;
    let existed = paths.settings.exists();
    let original = load_json_or_empty(&paths.settings)?;
    let content = existed.then(|| original.clone());
    write_backup(&BackupRecord { existed, content }, &paths.backup, files)?;

    create_settings_directory(&paths.settings)?;
    let merged = merge_claude_settings(
        &original,
        &MergeOptions {
            base_url,
            credential: &StaticToken { token: api_key },
            model: &ModelChoice::KeepModel,
            tier_model: None,
            status_line: Some(&status_line),
        },
    );
    write_settings(files, &paths.settings, &merged)
}

fn announce(stdout: &mut TerminalWriter, base_url: &str) -> Result<(), UpError> {
    writeln!(
        stdout,
        "litellm: routing Claude Code through proxy at {}",
        base_url.trim_end_matches('/')
    )
    .map_err(|source| UpError::Write { source })?;
    writeln!(
        stdout,
        "Press Ctrl-C to stop and restore your original settings."
    )
    .map_err(|source| UpError::Write { source })
}

fn verify_key<H, E, W>(
    ports: &UpPorts<'_, H, E, W>,
    base_url: &str,
    api_key: &SecretString,
) -> Result<(), UpError>
where
    H: Http + PollTransport + Send + Sync + 'static,
    E: Entropy,
    W: Clock + WallClock + Send + Sync + 'static,
{
    match verify_proxy_key(base_url, api_key, ports.probe) {
        Ok(()) => Ok(()),
        Err(AgentRunError::RejectedKey(status)) => Err(UpError::RejectedKey { status }),
        Err(AgentRunError::Unreachable(base, source)) => Err(UpError::Unreachable { base, source }),
        Err(other) => Err(UpError::Verify(other)),
    }
}

fn create_settings_directory(settings: &Path) -> Result<(), UpError> {
    let Some(directory) = settings.parent() else {
        return Ok(());
    };
    fs::create_dir_all(directory).map_err(|source| UpError::SettingsDirectory {
        path: settings.to_owned(),
        source,
    })
}

fn write_settings(
    files: &PrivateFiles,
    path: &Path,
    settings: &SettingsDocument,
) -> Result<(), UpError> {
    let written = block_on(write_claude_settings(files, path, settings))
        .map_err(|source| UpError::Runtime { source })?;
    written.map_err(UpError::Settings)
}

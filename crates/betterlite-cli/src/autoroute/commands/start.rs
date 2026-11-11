use std::fs;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use secrecy::SecretString;
use tokio::runtime::Builder;

use super::AutoroutePorts;
use super::error::CommandsError;
use super::master_key::ensure_master_key;
use super::teardown::{TeardownRequest, run_teardown};
use crate::autoroute::config::AUTOROUTER_MODEL_NAME;
use crate::autoroute::paths::{config_path, log_path, pid_record_path};
use crate::autoroute::process::control::terminate;
use crate::autoroute::process::launcher::{LaunchRequest, LaunchedProcess};
use crate::autoroute::process::liveliness::poll_liveliness;
use crate::autoroute::process::log_tail::TailWorker;
use crate::autoroute::process::pid_record::{
    PidRecord, clear_pid_record, read_pid_record, write_pid_record,
};
use crate::autoroute::process::runtime::gateway_runtime_available;
use crate::claude_settings::choices::{ModelChoice, StaticToken};
use crate::claude_settings::io::{load_json_or_empty, write_claude_settings, write_target};
use crate::claude_settings::merge::{MergeOptions, merge_claude_settings};
use crate::claude_settings::paths::claude_settings_paths;
use crate::claude_settings::statusline_install::install_statusline_script;
use crate::settings_backup::record::BackupRecord;
use crate::settings_backup::write::write_backup;

pub const RESERVED_PORT: u16 = 4000;

struct StartPaths {
    config: PathBuf,
    log: PathBuf,
    pid_record: PathBuf,
    settings: PathBuf,
    backup: PathBuf,
}

struct Launched {
    pid: NonZeroU32,
    base_url: String,
    _process: Box<dyn LaunchedProcess>,
}

pub fn start(ports: &mut AutoroutePorts<'_>, port: u16) -> Result<(), CommandsError> {
    let paths = start_paths(ports.home);
    let executable = preflight(ports, &paths, port)?;
    let master_key = ensure_master_key(&paths.config, ports.entropy)?;
    let launched = launch(ports, &paths, port, &executable)?;
    if let Err(error) = patch_settings(ports, &paths, &launched.base_url, &master_key) {
        abort(launched.pid.get(), ports, &paths.pid_record);
        return Err(error);
    }
    writeln!(
        ports.stdout,
        "betterlite: ephemeral auto-router proxy up at {} (pid {})",
        launched.base_url, launched.pid
    )
    .map_err(|source| CommandsError::Output { source })?;
    writeln!(
        ports.stdout,
        "Claude Code sessions started now will route through it. Press Ctrl-C to stop and restore."
    )
    .map_err(|source| CommandsError::Output { source })?;
    foreground(ports, &paths, launched.pid.get(), port)
}

fn start_paths(home: &Path) -> StartPaths {
    let settings = claude_settings_paths(home);
    StartPaths {
        config: config_path(home),
        log: log_path(home),
        pid_record: pid_record_path(home),
        settings: settings.settings,
        backup: settings.autoroute_backup,
    }
}

fn preflight(
    ports: &AutoroutePorts<'_>,
    paths: &StartPaths,
    port: u16,
) -> Result<PathBuf, CommandsError> {
    if !paths.config.exists() {
        return Err(CommandsError::MissingConfig);
    }
    let executable = gateway_runtime_available(ports.gateway)?;
    if let Some(record) = read_pid_record(&paths.pid_record)?
        && ports.signal_port.is_running(record.pid.get())
    {
        return Err(CommandsError::AlreadyRunning {
            pid: record.pid.get(),
        });
    }
    if paths.backup.exists() {
        return Err(CommandsError::BackupExists {
            path: paths.backup.clone(),
        });
    }
    if port == RESERVED_PORT {
        return Err(CommandsError::ReservedPort);
    }
    if !ports.port_probe.is_available(port) {
        return Err(CommandsError::PortInUse { port });
    }
    Ok(executable)
}

fn launch(
    ports: &AutoroutePorts<'_>,
    paths: &StartPaths,
    port: u16,
    executable: &Path,
) -> Result<Launched, CommandsError> {
    let base_url = format!("http://127.0.0.1:{port}");
    let arguments = gateway_arguments(&paths.config, port)?;
    let mut process = ports.launcher.launch(&LaunchRequest {
        executable,
        arguments: &arguments,
        log_path: &paths.log,
    })?;
    let pid = NonZeroU32::new(process.pid()).ok_or(CommandsError::MissingPid)?;
    let record = PidRecord {
        pid,
        port,
        config_path: paths.config.clone(),
        log_path: paths.log.clone(),
    };
    if let Err(error) = write_pid_record(&paths.pid_record, &record) {
        abort(pid.get(), ports, &paths.pid_record);
        return Err(CommandsError::Process(error));
    }
    if let Err(error) = poll_liveliness(
        &base_url,
        &paths.log,
        process.as_mut(),
        ports.liveliness,
        ports.clock,
    ) {
        abort(pid.get(), ports, &paths.pid_record);
        return Err(CommandsError::Process(error));
    }
    Ok(Launched {
        pid,
        base_url,
        _process: process,
    })
}

// The proxy is the gateway executable, with the config, port and loopback host LiteLLM passes
fn gateway_arguments(config: &Path, port: u16) -> Result<Vec<String>, CommandsError> {
    let config_text = config.to_str().ok_or_else(|| CommandsError::NonUtf8Path {
        path: config.to_owned(),
    })?;
    Ok(vec![
        "--config".to_owned(),
        config_text.to_owned(),
        "--port".to_owned(),
        port.to_string(),
        "--host".to_owned(),
        "127.0.0.1".to_owned(),
    ])
}

fn patch_settings(
    ports: &AutoroutePorts<'_>,
    paths: &StartPaths,
    base_url: &str,
    master_key: &SecretString,
) -> Result<(), CommandsError> {
    let status_line = install_statusline_script(ports.binary_path)?;
    let original_existed = paths.settings.exists();
    let original = load_json_or_empty(&paths.settings)?;
    let record = BackupRecord {
        existed: original_existed,
        content: original_existed.then(|| original.clone()),
    };
    write_backup(&record, &paths.backup, ports.files)?;
    let credential = StaticToken {
        token: master_key.clone(),
    };
    let model = ModelChoice::StartOn(AUTOROUTER_MODEL_NAME.to_owned());
    let options = MergeOptions {
        base_url,
        credential: &credential,
        model: &model,
        tier_model: Some(AUTOROUTER_MODEL_NAME),
        status_line: Some(&status_line),
    };
    let merged = merge_claude_settings(&original, &options);
    let target = write_target(&paths.settings)?;
    if let Some(directory) = target.parent() {
        fs::create_dir_all(directory).map_err(|source| CommandsError::SettingsDirectory {
            path: target.clone(),
            source,
        })?;
    }
    let runtime = Builder::new_current_thread()
        .build()
        .map_err(|source| CommandsError::Runtime { source })?;
    let written = runtime.block_on(write_claude_settings(ports.files, &target, &merged));
    written.map_err(CommandsError::Settings)?;
    Ok(())
}

fn foreground(
    ports: &mut AutoroutePorts<'_>,
    paths: &StartPaths,
    pid: u32,
    port: u16,
) -> Result<(), CommandsError> {
    let teardown = AtomicBool::new(false);
    let started = ports.signals.watch().and_then(|guard| {
        let tail = ports.tail.start(&paths.log)?;
        Ok((guard, tail))
    });
    let (guard, tail) = match started {
        Ok(ready) => ready,
        Err(error) => {
            let _ran = run_teardown(teardown_request(ports, paths, pid, port, None), &teardown);
            return Err(error);
        }
    };
    guard.wait();
    let _ran = run_teardown(
        teardown_request(ports, paths, pid, port, Some(tail)),
        &teardown,
    );
    Ok(())
}

fn teardown_request<'request>(
    ports: &'request mut AutoroutePorts<'_>,
    paths: &'request StartPaths,
    pid: u32,
    port: u16,
    tail: Option<Box<dyn TailWorker>>,
) -> TeardownRequest<'request> {
    TeardownRequest {
        settings: &paths.settings,
        backup: &paths.backup,
        pid_record: &paths.pid_record,
        pid,
        port,
        signal_port: ports.signal_port,
        clock: ports.clock,
        files: ports.files,
        tail,
        stdout: ports.stdout,
        stderr: ports.stderr,
    }
}

fn abort(pid: u32, ports: &AutoroutePorts<'_>, pid_record: &Path) {
    terminate(pid, ports.signal_port, ports.clock);
    let _cleared = clear_pid_record(pid_record);
}

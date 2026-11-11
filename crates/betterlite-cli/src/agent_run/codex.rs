use super::binary_name;
use super::error::AgentRunError;
use crate::agent_launch::{EnvVars, WindowsCommand, windows_command};
use crate::codex_catalog::{StockCodexCatalog, StockCodexModel};
use crate::opencode::ModelSyncSkipped;
use betterlite_logging::private_json::files::FileAccess;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::string::FromUtf8Error;
use std::time::Duration;
use tokio::process as tokio_process;
use tokio::runtime::Builder;
use tokio::time;
pub const CODEX_DEBUG_TIMEOUT: Duration = Duration::from_secs(10);
pub const CODEX_HOME_ENV: &str = "CODEX_HOME";
pub const CODEX_MODEL_CATALOG_FILENAME: &str = "litellm-models.json";
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DebugInvocation {
    pub command: WindowsCommand,
    pub env: EnvVars,
    pub timeout: Duration,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DebugOutput {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}
#[derive(Debug)]
pub enum DebugFailure {
    Io(io::Error),
    Timeout,
    Decode(FromUtf8Error),
}
pub trait DebugModels {
    fn run(&self, invocation: &DebugInvocation) -> Result<DebugOutput, DebugFailure>;
}
pub struct StdDebugModels;
impl DebugModels for StdDebugModels {
    fn run(&self, invocation: &DebugInvocation) -> Result<DebugOutput, DebugFailure> {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|source| DebugFailure::Io(io::Error::other(source)))?;
        runtime.block_on(run_debug_command(invocation))
    }
}
async fn run_debug_command(invocation: &DebugInvocation) -> Result<DebugOutput, DebugFailure> {
    let mut command = match &invocation.command {
        WindowsCommand::Direct(program, args) => {
            let mut command = tokio_process::Command::new(program);
            command.args(args);
            command
        }
        WindowsCommand::Batch(arguments) => batch_command(arguments)?,
    };
    command
        .env_clear()
        .envs(&invocation.env)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let output = match time::timeout(invocation.timeout, command.output()).await {
        Ok(result) => result.map_err(DebugFailure::Io)?,
        Err(_elapsed) => return Err(DebugFailure::Timeout),
    };
    Ok(DebugOutput {
        code: output.status.code().unwrap_or(-1_i32),
        stdout: String::from_utf8(output.stdout).map_err(DebugFailure::Decode)?,
        stderr: String::from_utf8(output.stderr).map_err(DebugFailure::Decode)?,
    })
}
fn batch_command(arguments: &str) -> Result<tokio_process::Command, DebugFailure> {
    #[cfg(windows)]
    {
        let mut command = tokio_process::Command::new("cmd.exe");
        command.raw_arg(arguments);
        Ok(command)
    }
    #[cfg(not(windows))]
    {
        Err(DebugFailure::Io(io::Error::other(format!(
            "cmd.exe is unavailable for `{arguments}`"
        ))))
    }
}
#[must_use]
pub fn codex_model_catalog_path(base_env: &EnvVars, home: Option<&Path>) -> Option<PathBuf> {
    if let Some(directory) = base_env
        .get(CODEX_HOME_ENV)
        .filter(|value| !value.is_empty())
    {
        return Some(Path::new(directory).join(CODEX_MODEL_CATALOG_FILENAME));
    }
    home.map(|base| base.join(".codex").join(CODEX_MODEL_CATALOG_FILENAME))
}
pub fn replace_file(path: &Path, text: &str, files: &dyn FileAccess) -> io::Result<()> {
    let directory = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut staged = files.create_staged(directory, None)?;
    if let Err(source) = files.write_staged(&mut staged, text.as_bytes()) {
        let _unused = files.remove(&staged.path);
        return Err(source);
    }
    if let Err(source) = files.replace(&staged.path, path) {
        let _unused = files.remove(&staged.path);
        return Err(source);
    }
    Ok(())
}
#[derive(Debug)]
pub enum DebugModelsOutcome {
    Printed(String),
    Skipped(ModelSyncSkipped),
}
pub fn codex_debug_models(
    binary: &str,
    args: &[String],
    env: &EnvVars,
    debug: &dyn DebugModels,
) -> Result<DebugModelsOutcome, AgentRunError> {
    let name = binary_name(binary);
    let mut command_args = vec![binary.to_owned()];
    command_args.extend(args.iter().cloned());
    command_args.push("debug".to_owned());
    command_args.push("models".to_owned());
    let command = windows_command(binary, &command_args)?;
    let invocation = DebugInvocation {
        command,
        env: env.clone(),
        timeout: CODEX_DEBUG_TIMEOUT,
    };
    let output = match debug.run(&invocation) {
        Ok(output) => output,
        Err(failure) => {
            return Ok(DebugModelsOutcome::Skipped(debug_failure(name, &failure)));
        }
    };
    if output.code == 0_i32 {
        return Ok(DebugModelsOutcome::Printed(output.stdout));
    }
    Ok(DebugModelsOutcome::Skipped(exit_failure(name, &output)))
}
fn debug_failure(name: &str, failure: &DebugFailure) -> ModelSyncSkipped {
    let reason = match failure {
        DebugFailure::Io(source) => format!("`{name} debug models` failed: {source}"),
        DebugFailure::Timeout => format!(
            "`{name} debug models` timed out after {} seconds",
            CODEX_DEBUG_TIMEOUT.as_secs()
        ),
        DebugFailure::Decode(source) => {
            format!("`{name} debug models` printed output that is not valid UTF-8: {source}")
        }
    };
    ModelSyncSkipped { reason }
}
fn exit_failure(name: &str, output: &DebugOutput) -> ModelSyncSkipped {
    let detail = output.stderr.trim().lines().next().unwrap_or("no output");
    ModelSyncSkipped {
        reason: format!("`{name} debug models` exited {}: {detail}", output.code),
    }
}
#[derive(Debug)]
pub enum StockModels {
    Models(Vec<StockCodexModel>),
    Skipped(ModelSyncSkipped),
}
pub fn stock_codex_models(
    binary: &str,
    env: &EnvVars,
    debug: &dyn DebugModels,
) -> Result<StockModels, AgentRunError> {
    match codex_debug_models(binary, &[], env, debug)? {
        DebugModelsOutcome::Skipped(skipped) => Ok(StockModels::Skipped(skipped)),
        DebugModelsOutcome::Printed(printed) => Ok(parse_stock_models(binary, &printed)),
    }
}
fn parse_stock_models(binary: &str, printed: &str) -> StockModels {
    match serde_json::from_str::<StockCodexCatalog>(printed) {
        Ok(catalog) => StockModels::Models(catalog.models),
        Err(source) => {
            let name = binary_name(binary);
            StockModels::Skipped(ModelSyncSkipped {
                reason: format!("`{name} debug models` printed no model catalog: {source}"),
            })
        }
    }
}

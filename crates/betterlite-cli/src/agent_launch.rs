use crate::cmd_quoting::quote_for_cmd;
use betterlite_config::loader::EnvironmentLookup;
use regex::Regex;
#[cfg(unix)]
use rustix::stdio::dup2_stdin;
use secrecy::{ExposeSecret as _, SecretString};
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{self, IsTerminal as _, Write};
#[cfg(unix)]
use std::os::unix::process::CommandExt as _;
#[cfg(windows)]
use std::os::windows::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process as std_process;
use std::time::Duration;
use thiserror::Error;

pub const LITELLM_PROXY_API_KEY_ENV: &str = "LITELLM_PROXY_API_KEY";
const GATEWAY_DISCOVERY: &str = "CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY";
const STOP_HOOKS_PATTERN: &str = r#"(?m)^\s*(\[\[\s*"?hooks"?\s*\.\s*"?Stop"?\s*\]\]|"?hooks"?(?:\s*\.\s*"?Stop"?)?\s*=|\[\s*"?hooks"?\s*\])"#;
pub type EnvVars = BTreeMap<String, String>;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvProfile {
    Anthropic,
    OpenAi,
    ProxyKey,
}
const BOTH_PROFILES: &[EnvProfile] = &[EnvProfile::Anthropic, EnvProfile::OpenAi];
pub const KNOWN_AGENTS: [(&str, &str, &[EnvProfile], bool); 4] = [
    ("claude", "Claude Code", &[EnvProfile::Anthropic], false),
    ("codex", "Codex", &[EnvProfile::OpenAi], false),
    ("opencode", "OpenCode", &[EnvProfile::OpenAi], false),
    ("pi", "pi", &[EnvProfile::ProxyKey], true),
];

fn command_name(command: &str) -> &str {
    Path::new(command)
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or(command)
}
#[must_use]
pub fn agent_profile(command: &str) -> (String, &'static [EnvProfile]) {
    let name = command_name(command);
    match KNOWN_AGENTS.iter().find(|(agent, _, _, _)| *agent == name) {
        Some((_, display_name, profiles, _)) => ((*display_name).to_owned(), *profiles),
        None => (name.to_owned(), BOTH_PROFILES),
    }
}

#[derive(Debug, Error)]
pub enum AgentRunError {
    #[error("could not reach the LiteLLM proxy at {0}: {1}. Is it running?")]
    Unreachable(String, #[source] io::Error),
    #[error("LiteLLM rejected your key (HTTP {0}); run `lite login` to refresh it")]
    RejectedKey(u16),
    #[error("cannot pass a line break to `{0}` on Windows: cmd.exe would drop the rest")]
    LineBreak(String),
    #[error("could not start the agent process: {0}")]
    Process(#[from] io::Error),
}
#[must_use]
pub fn build_agent_env(
    base_env: &EnvVars,
    base_url: &str,
    api_key: &SecretString,
    profiles: &[EnvProfile],
) -> EnvVars {
    let key = api_key.expose_secret();
    let root = base_url.trim_end_matches('/');
    let mut env = base_env.clone();
    if profiles.contains(&EnvProfile::Anthropic) {
        env.insert("ANTHROPIC_BASE_URL".to_owned(), root.to_owned());
        env.insert("ANTHROPIC_AUTH_TOKEN".to_owned(), key.to_owned());
        env.remove("ANTHROPIC_API_KEY");
        for (name, value) in [("ENABLE_TOOL_SEARCH", "true"), (GATEWAY_DISCOVERY, "1")] {
            env.entry(name.to_owned())
                .or_insert_with(|| value.to_owned());
        }
    }
    if profiles.contains(&EnvProfile::OpenAi) {
        env.insert("OPENAI_BASE_URL".to_owned(), format!("{root}/v1"));
        env.insert("OPENAI_API_KEY".to_owned(), key.to_owned());
    }
    if profiles.contains(&EnvProfile::ProxyKey) {
        env.insert(LITELLM_PROXY_API_KEY_ENV.to_owned(), key.to_owned());
    }
    env
}

pub struct CodexProxyProvider {
    pub name: &'static str,
    pub base_url: String,
    pub wire_api: &'static str,
    pub supports_websockets: bool,
    pub requires_openai_auth: bool,
}
#[must_use]
pub fn codex_proxy_provider(base_url: &str) -> CodexProxyProvider {
    CodexProxyProvider {
        name: "LiteLLM proxy",
        base_url: format!("{}/v1", base_url.trim_end_matches('/')),
        wire_api: "responses",
        supports_websockets: false,
        requires_openai_auth: false,
    }
}
#[must_use]
pub fn codex_proxy_args(base_url: &str) -> Vec<String> {
    let provider = codex_proxy_provider(base_url);
    let prefix = "model_providers.litellm";
    let websockets = provider.supports_websockets.to_string();
    let openai_auth = provider.requires_openai_auth.to_string();
    let fields = [
        ("name", serde_json::json!(provider.name).to_string()),
        ("base_url", serde_json::json!(provider.base_url).to_string()),
        ("wire_api", serde_json::json!(provider.wire_api).to_string()),
        ("supports_websockets", websockets),
        ("requires_openai_auth", openai_auth),
    ];
    let mut args = vec!["-c".to_owned(), "model_provider=\"litellm\"".to_owned()];
    for (key, value) in fields {
        args.extend(["-c".to_owned(), format!("{prefix}.{key}={value}")]);
    }
    let env_key = serde_json::json!("OPENAI_API_KEY").to_string();
    args.extend(["-c".to_owned(), format!("{prefix}.env_key={env_key}")]);
    args.extend(["-c".to_owned(), format!("{prefix}.http_headers={{}}")]);
    args
}

#[must_use]
pub fn agent_launch_args(command: &str, base_url: &str) -> Vec<String> {
    match command_name(command) {
        "codex" => codex_proxy_args(base_url),
        _ => Vec::new(),
    }
}
pub fn codex_config_path(environment: &dyn EnvironmentLookup) -> Option<PathBuf> {
    if let Some(home) = environment
        .lookup("CODEX_HOME")
        .filter(|home| !home.is_empty())
    {
        return Some(PathBuf::from(home).join("config.toml"));
    }
    let home = environment.system_home()?;
    Some(home.join(".codex").join("config.toml"))
}
#[must_use]
pub fn codex_declares_stop_hooks(config_path: &Path) -> bool {
    fs::read_to_string(config_path)
        .is_ok_and(|text| Regex::new(STOP_HOOKS_PATTERN).is_ok_and(|hooks| hooks.is_match(&text)))
}
pub fn warn(writer: &mut dyn Write, message: &str) -> io::Result<()> {
    writeln!(writer, "{message}")?;
    writer.flush()
}
pub trait KeyProbe {
    fn status(&self, url: &str, api_key: &SecretString, timeout: Duration) -> io::Result<u16>;
}
pub fn verify_proxy_key(
    base_url: &str,
    api_key: &SecretString,
    probe: &dyn KeyProbe,
) -> Result<(), AgentRunError> {
    let base = base_url.trim_end_matches('/');
    let url = format!("{base}/v1/models");
    let status = probe
        .status(&url, api_key, Duration::from_secs(10))
        .map_err(|source| AgentRunError::Unreachable(base.to_owned(), source))?;
    match status {
        401 | 403 => Err(AgentRunError::RejectedKey(status)),
        _ => Ok(()),
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WindowsCommand {
    Direct(String, Vec<String>),
    Batch(String),
}
pub fn windows_command(path: &str, args: &[String]) -> Result<WindowsCommand, AgentRunError> {
    let rest = args.iter().skip(1).cloned().collect::<Vec<String>>();
    let ext = Path::new(path).extension().and_then(OsStr::to_str);
    let is_shim = matches!(ext.map(str::to_lowercase).as_deref(), Some("cmd" | "bat"));
    if !is_shim {
        return Ok(WindowsCommand::Direct(path.to_owned(), rest));
    }
    if rest.iter().any(|token| token.contains(['\r', '\n'])) {
        return Err(AgentRunError::LineBreak(command_name(path).to_owned()));
    }
    let mut tokens = vec![quote_for_cmd(path)];
    tokens.extend(rest.iter().map(|token| quote_for_cmd(token)));
    let inner = tokens.join(" ");
    let arguments = format!("/d /e:on /v:off /s /c \"{inner}\"");
    Ok(WindowsCommand::Batch(arguments))
}
#[derive(Clone, Copy)]
pub enum Platform {
    Unix,
    Windows,
}
pub trait ProcessLauncher {
    fn replace(&self, path: &str, args: &[String], env: &EnvVars) -> io::Result<i32>;
    fn spawn(&self, command: &WindowsCommand, env: &EnvVars) -> io::Result<i32>;
}
pub struct StdLauncher;
#[cfg(unix)]
fn exec_command(path: &str, args: &[String], env: &EnvVars) -> std_process::Command {
    let mut command = std_process::Command::new(path);
    if let Some((name, rest)) = args.split_first() {
        command.arg0(name);
        command.args(rest);
    }
    command.env_clear().envs(env);
    command
}
impl ProcessLauncher for StdLauncher {
    #[cfg(unix)]
    fn replace(&self, path: &str, args: &[String], env: &EnvVars) -> io::Result<i32> {
        Err(exec_command(path, args, env).exec())
    }
    #[cfg(not(unix))]
    fn replace(&self, _path: &str, _args: &[String], _env: &EnvVars) -> io::Result<i32> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
    fn spawn(&self, command: &WindowsCommand, env: &EnvVars) -> io::Result<i32> {
        match command {
            WindowsCommand::Direct(program, arguments) => {
                let mut process = std_process::Command::new(program);
                process.args(arguments).env_clear().envs(env);
                let status = process.status()?;
                status.code().ok_or_else(|| io::Error::other("no code"))
            }
            WindowsCommand::Batch(arguments) => {
                #[cfg(windows)]
                {
                    let mut process = std_process::Command::new("cmd.exe");
                    process.raw_arg(arguments);
                    let status = process.env_clear().envs(env).status()?;
                    status.code().ok_or_else(|| io::Error::other("no code"))
                }
                #[cfg(not(windows))]
                {
                    Err(io::Error::other(format!("no cmd.exe for {arguments}")))
                }
            }
        }
    }
}
pub fn hand_off(
    path: &str,
    args: &[String],
    env: &EnvVars,
    platform: Platform,
    launcher: &dyn ProcessLauncher,
) -> Result<i32, AgentRunError> {
    let result = match platform {
        Platform::Unix => launcher.replace(path, args, env),
        Platform::Windows => launcher.spawn(&windows_command(path, args)?, env),
    };
    result.map_err(AgentRunError::Process)
}
pub trait Terminal {
    fn is_interactive(&self) -> bool;
    fn open_controlling(&self) -> io::Result<File>;
    fn attach_stdin(&self, terminal: File) -> io::Result<()>;
    fn restore_controlling_terminal(&self) -> io::Result<()> {
        if self.is_interactive() {
            return Ok(());
        }
        match self.open_controlling() {
            Ok(controlling) => self.attach_stdin(controlling),
            Err(_unavailable) => Ok(()),
        }
    }
}
pub struct StdTerminal;
impl Terminal for StdTerminal {
    fn is_interactive(&self) -> bool {
        io::stdin().is_terminal()
    }
    fn open_controlling(&self) -> io::Result<File> {
        File::open("/dev/tty")
    }
    fn attach_stdin(&self, terminal: File) -> io::Result<()> {
        #[cfg(not(unix))]
        drop(terminal);
        #[cfg(unix)]
        {
            dup2_stdin(&terminal).map_err(io::Error::from)
        }
        #[cfg(not(unix))]
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
}
mod tests;

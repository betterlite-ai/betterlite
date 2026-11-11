use betterlite_cli::agent_launch::{EnvVars, KeyProbe, ProcessLauncher, Terminal, WindowsCommand};
use betterlite_cli::agent_run::codex::{DebugFailure, DebugInvocation, DebugModels, DebugOutput};
use betterlite_cli::agent_run::messages::Messages;
use betterlite_cli::agent_run::prepare::StatuslineInstaller;
use betterlite_cli::agent_run::resolver::BinaryResolver;
use betterlite_cli::claude_settings::error::ClaudeSettingsError;
use betterlite_cli::opencode::{ModelListingRequest, ModelListingResponse, ModelListingTransport};
use betterlite_cli::pi::{ProxyGet, ProxyResponse};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::fs::File;
use std::io;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DebugScript {
    pub(crate) status: i32,
    #[serde(default)]
    pub(crate) stdout: String,
    #[serde(default)]
    pub(crate) stderr: String,
}
pub(crate) struct Http {
    pub(crate) rows: Vec<Value>,
}
impl Http {
    fn response(&self, url: &str) -> (u16, Vec<u8>) {
        if !url.ends_with("/v1/models") {
            return (404, Vec::new());
        }
        match serde_json::to_vec(&json!({"data": &self.rows})) {
            Ok(body) => (200, body),
            Err(_invalid) => (500, Vec::new()),
        }
    }
}
impl ProxyGet for Http {
    fn get(
        &self,
        url: &str,
        headers: &[(String, SecretString)],
        timeout: Duration,
    ) -> io::Result<ProxyResponse> {
        if !headers.iter().any(|(name, _value)| name == "Authorization") || timeout.is_zero() {
            return Err(io::Error::from(io::ErrorKind::InvalidInput));
        }
        let (status, body) = self.response(url);
        Ok(ProxyResponse { status, body })
    }
}
impl ModelListingTransport for Http {
    fn get(&self, request: &ModelListingRequest) -> io::Result<ModelListingResponse> {
        if request.authorization.expose_secret().is_empty() || request.timeout.is_zero() {
            return Err(io::Error::from(io::ErrorKind::InvalidInput));
        }
        let (status, body) = self.response(&request.url);
        Ok(ModelListingResponse { status, body })
    }
}
pub(crate) struct Probe;
impl KeyProbe for Probe {
    fn status(&self, url: &str, api_key: &SecretString, timeout: Duration) -> io::Result<u16> {
        if !url.ends_with("/v1/models") || api_key.expose_secret().is_empty() || timeout.is_zero() {
            return Err(io::Error::from(io::ErrorKind::InvalidInput));
        }
        Ok(200)
    }
}
pub(crate) struct Debug {
    pub(crate) answers: Mutex<VecDeque<DebugScript>>,
}
impl DebugModels for Debug {
    fn run(&self, invocation: &DebugInvocation) -> Result<DebugOutput, DebugFailure> {
        let WindowsCommand::Direct(_program, _args) = &invocation.command else {
            return Err(DebugFailure::Io(io::Error::other(
                "unexpected batch debug command",
            )));
        };
        if !invocation.env.contains_key("CODEX_HOME") {
            return Err(DebugFailure::Io(io::Error::other(
                "the debug command ran without the launch environment",
            )));
        }
        let answer = self
            .answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        match answer {
            Some(script) => Ok(DebugOutput {
                code: script.status,
                stdout: script.stdout,
                stderr: script.stderr,
            }),
            None => Err(DebugFailure::Io(io::Error::other(
                "no scripted debug answer",
            ))),
        }
    }
}
#[derive(Default)]
pub(crate) struct Recorder {
    pub(crate) warns: Vec<String>,
    pub(crate) notices: Vec<String>,
}
impl Messages for Recorder {
    fn warn(&mut self, message: &str) -> io::Result<()> {
        self.warns.push(message.to_owned());
        Ok(())
    }
    fn notice(&mut self, message: &str) -> io::Result<()> {
        self.notices.push(message.to_owned());
        Ok(())
    }
}
pub(crate) struct SilentTerminal;
impl Terminal for SilentTerminal {
    fn is_interactive(&self) -> bool {
        false
    }
    fn open_controlling(&self) -> io::Result<File> {
        Err(io::Error::from(io::ErrorKind::NotFound))
    }
    fn attach_stdin(&self, terminal: File) -> io::Result<()> {
        let _unused = terminal;
        Ok(())
    }
    fn restore_controlling_terminal(&self) -> io::Result<()> {
        Ok(())
    }
}
#[derive(Clone)]
pub(crate) struct LaunchCall {
    pub(crate) path: String,
    pub(crate) args: Vec<String>,
    pub(crate) env: EnvVars,
}
pub(crate) struct Launcher {
    pub(crate) calls: Mutex<Vec<LaunchCall>>,
}

impl ProcessLauncher for Launcher {
    fn replace(&self, path: &str, args: &[String], env: &EnvVars) -> io::Result<i32> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(LaunchCall {
                path: path.to_owned(),
                args: args.to_vec(),
                env: env.clone(),
            });
        Ok(0)
    }
    fn spawn(&self, command: &WindowsCommand, env: &EnvVars) -> io::Result<i32> {
        Err(io::Error::other(format!(
            "spawn is not scripted: {command:?} with {} vars",
            env.len()
        )))
    }
}
pub(crate) struct Resolver(pub(crate) Option<String>);
impl BinaryResolver for Resolver {
    fn resolve(&self, command: &str) -> Option<String> {
        if command.is_empty() {
            return None;
        }
        self.0.clone()
    }
}
pub(crate) struct Installer;
impl StatuslineInstaller for Installer {
    fn install(&self) -> Result<String, ClaudeSettingsError> {
        Ok(super::INSTALL_COMMAND.to_owned())
    }
}

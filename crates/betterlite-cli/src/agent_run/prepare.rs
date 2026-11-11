use super::error::AgentRunError;
use super::messages::{Messages, NoticeBuffer};
use crate::agent_launch::{codex_config_path, codex_declares_stop_hooks};
use crate::claude_settings::error::ClaudeSettingsError;
use crate::claude_settings::statusline_install::install_statusline_script;
use crate::pi::ProxyGet;
use crate::pi::prepare::{PreparePiContext, prepare_pi};
use betterlite_config::loader::EnvironmentLookup;
use betterlite_logging::private_json::files::FileAccess;
use secrecy::SecretString;
pub struct PrepareRequest<'request> {
    pub base_url: &'request str,
    pub api_key: &'request SecretString,
    pub environment: &'request dyn EnvironmentLookup,
}
pub trait Preparer {
    fn prepare(
        &self,
        request: &PrepareRequest<'_>,
        messages: &mut dyn Messages,
    ) -> Result<Vec<String>, AgentRunError>;
}
pub trait StatuslineInstaller {
    fn install(&self) -> Result<String, ClaudeSettingsError>;
}
pub struct StdStatuslineInstaller;
impl StatuslineInstaller for StdStatuslineInstaller {
    fn install(&self) -> Result<String, ClaudeSettingsError> {
        install_statusline_script(None)
    }
}
pub struct CodexPreparer<'installer> {
    pub install: &'installer dyn StatuslineInstaller,
}
impl Preparer for CodexPreparer<'_> {
    fn prepare(
        &self,
        request: &PrepareRequest<'_>,
        messages: &mut dyn Messages,
    ) -> Result<Vec<String>, AgentRunError> {
        let Some(config_path) = codex_config_path(request.environment) else {
            return install_stop_hook(self.install);
        };
        if codex_declares_stop_hooks(&config_path) {
            let _unused = messages.warn(
                "litellm: your Codex config already declares hooks; not adding the routed-model Stop hook",
            );
            return Ok(Vec::new());
        }
        install_stop_hook(self.install)
    }
}
fn install_stop_hook(install: &dyn StatuslineInstaller) -> Result<Vec<String>, AgentRunError> {
    let command = install.install()?;
    let quoted = serde_json::json!(command);
    Ok(vec![
        "-c".to_owned(),
        format!("hooks.Stop=[{{hooks=[{{type=\"command\",command={quoted}}}]}}]"),
    ])
}
pub struct PiPreparer<'dependencies> {
    pub get: &'dependencies dyn ProxyGet,
    pub files: &'dependencies dyn FileAccess,
}
impl Preparer for PiPreparer<'_> {
    fn prepare(
        &self,
        request: &PrepareRequest<'_>,
        messages: &mut dyn Messages,
    ) -> Result<Vec<String>, AgentRunError> {
        prepare_pi_models(self, request, messages)
    }
}
fn prepare_pi_models(
    pi: &PiPreparer<'_>,
    request: &PrepareRequest<'_>,
    messages: &mut dyn Messages,
) -> Result<Vec<String>, AgentRunError> {
    let mut notice = NoticeBuffer {
        messages,
        text: String::new(),
    };
    let mut context = PreparePiContext {
        environment: request.environment,
        get: pi.get,
        files: pi.files,
        notice: &mut notice,
    };
    let result = prepare_pi(request.base_url, request.api_key, &mut context);
    notice.finish();
    result.map_err(AgentRunError::Pi)
}
pub struct Preparers<'preparers> {
    pub pi: &'preparers dyn Preparer,
    pub codex: &'preparers dyn Preparer,
}
impl Preparers<'_> {
    #[must_use]
    pub fn get(&self, command: &str) -> Option<&dyn Preparer> {
        match command {
            "pi" => Some(self.pi),
            "codex" => Some(self.codex),
            _ => None,
        }
    }
}

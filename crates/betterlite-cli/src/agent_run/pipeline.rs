use super::binary_name;
use super::environment::BaseEnvironment;
use super::error::AgentRunError;
use super::messages::Messages;
use super::prepare::{PrepareRequest, Preparers};
use super::resolver::BinaryResolver;
use super::sync::{ModelSync, SyncRequest};
use crate::agent_launch::{
    EnvProfile, EnvVars, KeyProbe, Platform, ProcessLauncher, Terminal, agent_launch_args,
    agent_profile, build_agent_env, hand_off, verify_proxy_key,
};
use crate::opencode::ModelSyncResult;
use secrecy::SecretString;
use std::path::Path;
const INSTALL_DOCS: [(&str, &str); 4] = [
    (
        "claude",
        "https://docs.claude.com/en/docs/claude-code/setup",
    ),
    ("codex", "https://developers.openai.com/codex/cli"),
    ("opencode", "https://opencode.ai/docs"),
    ("pi", "https://pi.dev"),
];
pub struct RunAgentRequest<'request> {
    pub base_url: &'request str,
    pub api_key: &'request SecretString,
    pub command: &'request [String],
    pub skip_verify: bool,
    pub base_env: &'request EnvVars,
    pub home: Option<&'request Path>,
    pub resolver: &'request dyn BinaryResolver,
    pub probe: &'request dyn KeyProbe,
    pub sync: &'request dyn ModelSync,
    pub preparers: &'request Preparers<'request>,
    pub launcher: &'request dyn ProcessLauncher,
    pub platform: Platform,
    pub terminal: Option<&'request dyn Terminal>,
    pub messages: &'request mut dyn Messages,
}
pub fn run_agent(request: RunAgentRequest<'_>) -> Result<i32, AgentRunError> {
    let Some(program) = request.command.first() else {
        return Err(AgentRunError::NothingToRun);
    };
    let base_env = request.base_env;
    let base_url = request.base_url;
    let api_key = request.api_key;
    let (display_name, profiles) = agent_profile(program);
    let messages = request.messages;
    let binary = resolve_binary(program, request.resolver)?;
    if !request.skip_verify {
        verify_proxy_key(base_url, api_key, request.probe)?;
    }
    let sync_request = SyncRequest {
        binary: &binary,
        base_env,
        base_url,
        api_key,
        skip_verify: request.skip_verify,
    };
    let synced = request.sync.sync(&sync_request)?;
    warn_skipped(&synced, &display_name, messages);
    let environment = BaseEnvironment {
        variables: base_env,
        home: request.home,
    };
    let prepare_request = PrepareRequest {
        base_url,
        api_key,
        environment: &environment,
    };
    let prepared = run_preparer(request.preparers, program, &prepare_request, messages)?;
    let env = compose_env(base_env, base_url, api_key, profiles, &synced);
    let argv = command_line(program, base_url, request.command, &synced, prepared);
    reattach(request.terminal)?;
    hand_off(&binary, &argv, &env, request.platform, request.launcher)
        .map_err(AgentRunError::Launch)
}
fn run_preparer(
    preparers: &Preparers<'_>,
    program: &str,
    request: &PrepareRequest<'_>,
    messages: &mut dyn Messages,
) -> Result<Vec<String>, AgentRunError> {
    preparers.get(binary_name(program)).map_or_else(
        || Ok(Vec::new()),
        |preparer| preparer.prepare(request, messages),
    )
}
fn command_line(
    program: &str,
    base_url: &str,
    command: &[String],
    synced: &ModelSyncResult,
    prepared: Vec<String>,
) -> Vec<String> {
    let mut argv = vec![program.to_owned()];
    argv.extend(agent_launch_args(program, base_url));
    if let ModelSyncResult::Args(sync_args) = synced {
        argv.extend(sync_args.args.iter().cloned());
    }
    argv.extend(prepared);
    argv.extend(command.iter().skip(1).cloned());
    argv
}
fn reattach(terminal: Option<&dyn Terminal>) -> Result<(), AgentRunError> {
    terminal.map_or_else(
        || Ok(()),
        |controlling| {
            controlling
                .restore_controlling_terminal()
                .map_err(|source| AgentRunError::Terminal { source })
        },
    )
}
fn resolve_binary(program: &str, resolver: &dyn BinaryResolver) -> Result<String, AgentRunError> {
    resolver.resolve(program).map_or_else(
        || {
            Err(AgentRunError::MissingBinary {
                command: program.to_owned(),
                hint: install_hint(program),
            })
        },
        Ok,
    )
}
fn install_hint(program: &str) -> String {
    let name = binary_name(program);
    match INSTALL_DOCS.iter().find(|(agent, _docs)| *agent == name) {
        Some((_agent, docs)) => format!(" Install it first: {docs}"),
        None => String::new(),
    }
}
fn warn_skipped(synced: &ModelSyncResult, display_name: &str, messages: &mut dyn Messages) {
    if let ModelSyncResult::Skipped(skipped) = synced {
        let _unused = messages.warn(&format!(
            "litellm: not syncing {display_name} models from the proxy: {}",
            skipped.reason
        ));
    }
}
fn compose_env(
    base_env: &EnvVars,
    base_url: &str,
    api_key: &SecretString,
    profiles: &[EnvProfile],
    synced: &ModelSyncResult,
) -> EnvVars {
    let mut env = build_agent_env(base_env, base_url, api_key, profiles);
    if let ModelSyncResult::Env(extra) = synced {
        env.extend(extra.clone());
    }
    env
}

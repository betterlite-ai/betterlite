use std::collections::{BTreeMap, BTreeSet};

use secrecy::SecretString;

use crate::context::{GatewayTarget, SessionContext};
use crate::errors::HarnessError;
use crate::events::Event;
use crate::options::ClaudeCodeOptions;
use crate::sandbox::Sandbox;
use crate::transformation::{
    BaseCLIHarnessConfig, BaseHarnessConfig, HarnessSessionSetup, HarnessTurnRequest,
    HarnessTurnResponse, PersistedDir, TurnInput,
};
use crate::types::{Capabilities, Capability, Harness, JsonObject, PermissionMode};

#[path = "claude_code/config.rs"]
mod config;
#[path = "claude_code/stream.rs"]
pub mod stream;
mod tests;

use config::{BASE_COMMAND, CLAUDE_BINARY, STATIC_ENV};
use stream::ClaudeCodeStreamState;

fn session_env(
    options: &ClaudeCodeOptions,
    gateway: &GatewayTarget,
    private_dir: &str,
    model: Option<&str>,
) -> BTreeMap<String, SecretString> {
    let mut env = options
        .env
        .iter()
        .map(|(key, value)| (key.clone(), SecretString::from(value.clone())))
        .collect::<BTreeMap<_, _>>();
    env.extend(
        STATIC_ENV
            .iter()
            .map(|(key, value)| ((*key).to_owned(), SecretString::from((*value).to_owned()))),
    );
    env.insert(
        "ANTHROPIC_BASE_URL".to_owned(),
        SecretString::from(gateway.api_base.clone()),
    );
    env.insert("ANTHROPIC_AUTH_TOKEN".to_owned(), gateway.api_key.clone());
    env.insert(
        "ANTHROPIC_API_KEY".to_owned(),
        SecretString::from(String::new()),
    );
    env.insert(
        "CLAUDE_CONFIG_DIR".to_owned(),
        SecretString::from(private_dir.to_owned()),
    );
    if let Some(model_name) = model {
        env.insert(
            "ANTHROPIC_MODEL".to_owned(),
            SecretString::from(model_name.to_owned()),
        );
        env.insert(
            "ANTHROPIC_SMALL_FAST_MODEL".to_owned(),
            SecretString::from(model_name.to_owned()),
        );
    }
    env
}

fn turn_argv<S>(
    context: &SessionContext<S>,
    options: &ClaudeCodeOptions,
    native_session_id: Option<&str>,
) -> Result<Vec<String>, HarnessError> {
    let mut argv = Vec::new();
    if !options.config.is_empty() {
        argv.extend([
            "--settings".to_owned(),
            serde_json::to_string(&options.config)
                .map_err(|source| HarnessError::StateEncode { source })?,
        ]);
    }
    let system_prompt = config::build_system_prompt(
        context.settings.instructions.as_deref(),
        context.settings.output_schema.as_ref(),
    )?;
    if let Some(prompt) = system_prompt {
        argv.extend(["--append-system-prompt".to_owned(), prompt]);
    }
    if let Some(max_turns) = context.settings.max_turns {
        argv.extend(["--max-turns".to_owned(), max_turns.to_string()]);
    }
    let disallowed = config::native_tool_names(&context.settings.disable_tools);
    if !disallowed.is_empty() {
        argv.extend(["--disallowedTools".to_owned(), disallowed.join(",")]);
    }
    if let Some(session_id) = native_session_id {
        argv.extend(["--resume".to_owned(), session_id.to_owned()]);
    }
    Ok(argv)
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ClaudeCodeHarnessConfig;

impl BaseHarnessConfig for ClaudeCodeHarnessConfig {
    type Options = ClaudeCodeOptions;
    const HARNESS: Harness = Harness::ClaudeCode;

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            supported: BTreeSet::from([
                Capability::StructuredOutput,
                Capability::ToolFiltering,
                Capability::Skills,
                Capability::Resume,
            ]),
            permission_modes: BTreeSet::from([
                PermissionMode::ReadOnly,
                PermissionMode::Edit,
                PermissionMode::Full,
            ]),
        }
    }

    fn validate_environment<S>(&self, context: &SessionContext<S>) -> Result<(), HarnessError> {
        let options = self.get_options(context)?;
        config::validate_options(&options)
    }
}

impl BaseCLIHarnessConfig for ClaudeCodeHarnessConfig {
    type StreamState = ClaudeCodeStreamState;

    fn get_binary(&self) -> &'static str {
        CLAUDE_BINARY
    }
    fn get_install_hint(&self) -> &'static str {
        "npm install -g @anthropic-ai/claude-code"
    }

    fn transform_session_setup<S>(
        &self,
        context: &SessionContext<S>,
        private_dir: &str,
    ) -> Result<HarnessSessionSetup, HarnessError>
    where
        S: Sandbox,
    {
        let gateway = context
            .settings
            .gateway
            .as_ref()
            .ok_or(HarnessError::OptionsMismatch {
                given: "SessionSettings.gateway",
                harness: Harness::ClaudeCode,
                expected: "a configured session model endpoint",
            })?;
        let options = self.get_options(context)?;
        config::validate_options(&options)?;
        let env = session_env(
            &options,
            gateway,
            private_dir,
            context.settings.model.as_deref(),
        );
        Ok(HarnessSessionSetup {
            files: BTreeMap::new(),
            persisted_dirs: vec![PersistedDir {
                private_path: "projects".to_owned(),
                cache_path: "claude_code/projects".to_owned(),
            }],
            skills_dir: Some("skills".to_owned()),
            env,
        })
    }

    fn transform_turn_request<S>(
        &self,
        context: &SessionContext<S>,
        turn: TurnInput<'_>,
    ) -> Result<HarnessTurnRequest, HarnessError>
    where
        S: Sandbox,
    {
        let options = self.get_options(context)?;
        let mut argv = BASE_COMMAND
            .iter()
            .map(|value| (*value).to_owned())
            .collect::<Vec<_>>();
        argv.insert(0, CLAUDE_BINARY.to_owned());
        argv.extend([
            "--permission-mode".to_owned(),
            config::permission_mode(context.settings.permissions).to_owned(),
        ]);
        if let Some(model) = context.settings.model.as_ref() {
            argv.extend(["--model".to_owned(), model.clone()]);
        }
        argv.extend(["--setting-sources".to_owned(), "user".to_owned()]);
        argv.extend(turn_argv(context, &options, turn.native_session_id)?);
        Ok(HarnessTurnRequest {
            argv,
            env: turn.setup.env.clone(),
            stdin: turn.prompt.to_owned(),
            cwd: None,
        })
    }

    fn create_stream_state(&self) -> Self::StreamState {
        ClaudeCodeStreamState::default()
    }
    fn transform_stream_line(
        &self,
        line: &JsonObject,
        state: &mut Self::StreamState,
    ) -> Vec<Event> {
        stream::transform(line, state)
    }
    fn get_native_session_id(&self, state: &Self::StreamState) -> Option<String> {
        state.session_id.clone()
    }

    fn transform_turn_response<S>(
        &self,
        context: &SessionContext<S>,
        state: &Self::StreamState,
        exit_code: i32,
        stderr_tail: &[String],
    ) -> Result<HarnessTurnResponse, HarnessError> {
        if let Some(reason) = stream::turn_error(state, exit_code, stderr_tail) {
            return Err(HarnessError::TurnFailed { reason });
        }
        let output_json = context
            .settings
            .output_schema
            .as_ref()
            .map(|_| state.structured_output_json());
        Ok(HarnessTurnResponse {
            final_text: state.final_text(),
            output_json,
        })
    }
}

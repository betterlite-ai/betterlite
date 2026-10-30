#![expect(
    clippy::disallowed_types,
    reason = "opencode JSON config and stream lines are untyped"
)]

use std::collections::BTreeMap;

use secrecy::{ExposeSecret as _, SecretString};

use crate::context::{GatewayTarget, SessionContext};
use crate::errors::HarnessError;
use crate::events::Event;
use crate::options::OpenCodeOptions;
use crate::payloads::python_json::python_json_text;
use crate::payloads::{last_json_object, stderr_tail_text};
use crate::sandbox::Sandbox;
use crate::transformation::{
    BaseCLIHarnessConfig, BaseHarnessConfig as _, HarnessSessionSetup, HarnessTurnRequest,
    HarnessTurnResponse, PersistedDir, TurnInput,
};
use crate::types::{Harness, JsonObject};

#[path = "capabilities.rs"]
mod capabilities;
#[path = "config.rs"]
mod config;
#[path = "permissions.rs"]
mod permissions;
#[path = "stream.rs"]
pub mod stream;
mod tests;
#[path = "wire.rs"]
mod wire;

use config::{
    ConfigInputs, INSTRUCTIONS_FILENAME, OPENCODE_BINARY, OPENCODE_ISOLATION_ENV,
    OPENCODE_PROVIDER_ID, OPENCODE_SESSION_TITLE, TOKEN_FILENAME, XDG_DIRNAME, XDG_SUBDIRS,
    build_instructions, build_opencode_config, turn_prompt,
};
use stream::OpenCodeStreamState;

#[derive(Clone, Copy, Debug, Default)]
pub struct OpenCodeHarnessConfig;

const fn session_gateway<S>(context: &SessionContext<S>) -> Result<&GatewayTarget, HarnessError> {
    let Some(gateway) = context.settings.gateway.as_ref() else {
        return Err(HarnessError::OptionsMismatch {
            given: "SessionSettings.gateway",
            harness: Harness::OpenCode,
            expected: "a configured session model endpoint",
        });
    };
    Ok(gateway)
}

fn session_files(gateway_token: &[u8], instructions: Option<&String>) -> BTreeMap<String, Vec<u8>> {
    let mut files = BTreeMap::new();
    files.insert(TOKEN_FILENAME.to_owned(), gateway_token.to_vec());
    if let Some(text) = instructions {
        files.insert(INSTRUCTIONS_FILENAME.to_owned(), text.clone().into_bytes());
    }
    files
}

fn session_config_text<S>(
    context: &SessionContext<S>,
    model: &str,
    options: &OpenCodeOptions,
    instructions: Option<&String>,
    private_dir: &str,
) -> Result<String, HarnessError> {
    let gateway = session_gateway(context)?;
    let base_url = format!("{}/v1", gateway.api_base.trim_end_matches('/'));
    let config = build_opencode_config(&ConfigInputs {
        model,
        base_url,
        token_path: format!("{private_dir}/{TOKEN_FILENAME}"),
        permissions: context.settings.permissions,
        disable_tools: &context.settings.disable_tools,
        user_config: &options.config,
        instructions_path: instructions
            .is_some()
            .then(|| format!("{private_dir}/{INSTRUCTIONS_FILENAME}")),
        skills_path: (!context.settings.skills.is_empty()).then(|| format!("{private_dir}/skills")),
    })?;
    python_json_text(&config).map_err(|source| HarnessError::StateEncode { source })
}

fn session_env(
    options_env: &BTreeMap<String, String>,
    private_dir: &str,
    config_text: String,
) -> BTreeMap<String, SecretString> {
    let mut env: BTreeMap<String, SecretString> = OPENCODE_ISOLATION_ENV
        .iter()
        .map(|(key, value)| ((*key).to_owned(), SecretString::from((*value).to_owned())))
        .collect();
    env.extend(
        options_env
            .iter()
            .map(|(key, value)| (key.clone(), SecretString::from(value.clone()))),
    );
    for sub in XDG_SUBDIRS {
        env.insert(
            format!("XDG_{}_HOME", sub.to_uppercase()),
            SecretString::from(format!("{private_dir}/{XDG_DIRNAME}/{sub}")),
        );
    }
    env.insert(
        "OPENCODE_CONFIG_CONTENT".to_owned(),
        SecretString::from(config_text),
    );
    env
}

impl BaseCLIHarnessConfig for OpenCodeHarnessConfig {
    type StreamState = OpenCodeStreamState;

    fn get_binary(&self) -> &'static str {
        OPENCODE_BINARY
    }

    fn get_install_hint(&self) -> &'static str {
        "npm install -g opencode-ai (or brew install sst/tap/opencode)"
    }

    // the gateway stand-in and its token-file isolation mirror B4835; see docs/deviations/B4836.yaml
    fn transform_session_setup<S>(
        &self,
        context: &SessionContext<S>,
        private_dir: &str,
    ) -> Result<HarnessSessionSetup, HarnessError>
    where
        S: Sandbox,
    {
        let gateway = session_gateway(context)?;
        let model = context
            .settings
            .model
            .clone()
            .ok_or(HarnessError::OptionsMismatch {
                given: "SessionSettings.model",
                harness: Harness::OpenCode,
                expected: "a model name",
            })?;
        let options = self.get_options(context)?;
        let instructions = build_instructions(context)?;
        let files = session_files(
            gateway.api_key.expose_secret().as_bytes(),
            instructions.as_ref(),
        );
        let config_text = session_config_text(
            context,
            &model,
            &options,
            instructions.as_ref(),
            private_dir,
        )?;
        let env = session_env(&options.env, private_dir, config_text);
        Ok(HarnessSessionSetup {
            files,
            persisted_dirs: vec![PersistedDir {
                private_path: XDG_DIRNAME.to_owned(),
                cache_path: "opencode".to_owned(),
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
        let model = context
            .settings
            .model
            .clone()
            .ok_or(HarnessError::OptionsMismatch {
                given: "SessionSettings.model",
                harness: Harness::OpenCode,
                expected: "a model name",
            })?;
        let options = self.get_options(context)?;
        let mut argv = vec![
            OPENCODE_BINARY.to_owned(),
            "run".to_owned(),
            "--pure".to_owned(),
            "--format".to_owned(),
            "json".to_owned(),
            "--thinking".to_owned(),
            "-m".to_owned(),
            format!("{OPENCODE_PROVIDER_ID}/{model}"),
        ];
        if !options.agent.is_empty() {
            argv.extend(["--agent".to_owned(), options.agent]);
        }
        match turn.native_session_id {
            Some(id) => argv.extend(["--session".to_owned(), id.to_owned()]),
            None => argv.extend(["--title".to_owned(), OPENCODE_SESSION_TITLE.to_owned()]),
        }
        Ok(HarnessTurnRequest {
            argv,
            env: turn.setup.env.clone(),
            stdin: turn_prompt(context, turn.prompt)?,
            cwd: Some(context.sandbox.workdir().to_owned()),
        })
    }

    fn create_stream_state(&self) -> Self::StreamState {
        OpenCodeStreamState::default()
    }

    fn transform_stream_line(
        &self,
        line: &JsonObject,
        state: &mut Self::StreamState,
    ) -> Vec<Event> {
        stream::transform_line(line, state)
    }

    fn get_native_session_id(&self, state: &Self::StreamState) -> Option<String> {
        state.session_id.clone()
    }

    // an error event fails the turn even at exit code 0; see docs/deviations/B4836.yaml
    fn transform_turn_response<S>(
        &self,
        context: &SessionContext<S>,
        state: &Self::StreamState,
        exit_code: i32,
        stderr_tail: &[String],
    ) -> Result<HarnessTurnResponse, HarnessError> {
        if let Some(error) = state.error.as_ref() {
            return Err(HarnessError::TurnFailed {
                reason: format!("opencode reported an error: {error}"),
            });
        }
        if exit_code != 0_i32 {
            let tail = stderr_tail_text(stderr_tail);
            let detail = if tail.is_empty() {
                "no output".to_owned()
            } else {
                tail
            };
            return Err(HarnessError::TurnFailed {
                reason: format!("opencode exited with code {exit_code}: {detail}"),
            });
        }
        Ok(HarnessTurnResponse {
            final_text: state.final_text.clone(),
            output_json: context
                .settings
                .output_schema
                .as_ref()
                .and_then(|_| last_json_object(&state.final_text)),
        })
    }
}

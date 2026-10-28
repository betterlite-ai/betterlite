use secrecy::SecretString;
use std::collections::BTreeMap;

use crate::context::SessionContext;
use crate::errors::HarnessError;
use crate::events::Event;
use crate::payloads::{stderr_tail_text, strict_json_schema};
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
#[path = "features.rs"]
mod features;
#[path = "permissions.rs"]
mod permissions;
#[path = "provider.rs"]
mod provider;
#[path = "stream.rs"]
pub mod stream;
#[path = "stream_events.rs"]
mod stream_events;
mod tests;
use config::config_overrides;
use config::{CODEX_HOME_ENV, CODEX_SCHEMA_FILENAME, CODEX_TOKEN_ENV};
use features::feature_overrides;
use permissions::permission_args;
use provider::provider_overrides;
use stream::{CodexStreamState, ItemEventKind, StreamEvent};
use stream_events::{record_thread_id, record_turn_failed, transform_item_event};

#[derive(Clone, Copy, Debug, Default)]
pub struct CodexHarnessConfig;

fn reject_managed_env_collision(env: &BTreeMap<String, String>) -> Result<(), HarnessError> {
    if env.contains_key(CODEX_TOKEN_ENV) || env.contains_key(CODEX_HOME_ENV) {
        return Err(HarnessError::OptionsMismatch {
            given: "CodexOptions.env",
            harness: Harness::Codex,
            expected: "keys other than LITELLM_HARNESS_TOKEN or CODEX_HOME",
        });
    }
    Ok(())
}

fn output_schema_file(schema: &JsonObject) -> Result<Vec<u8>, HarnessError> {
    let strict = strict_json_schema(config::schema_value(schema), 0).map_err(|_schema_err| {
        HarnessError::OptionsMismatch {
            given: "SessionSettings.output_schema",
            harness: Harness::Codex,
            expected: "a JSON Schema within the Codex config nesting limit",
        }
    })?;
    serde_json::to_vec(&strict).map_err(|source| HarnessError::StateEncode { source })
}

impl BaseCLIHarnessConfig for CodexHarnessConfig {
    type StreamState = CodexStreamState;
    fn get_binary(&self) -> &'static str {
        "codex"
    }
    fn get_install_hint(&self) -> &'static str {
        "npm install -g @openai/codex (or brew install codex)"
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
                harness: Harness::Codex,
                expected: "a configured session model endpoint",
            })?;
        let options = self.get_options(context)?;
        reject_managed_env_collision(&options.env)?;
        let mut setup = HarnessSessionSetup {
            files: BTreeMap::new(),
            persisted_dirs: vec![PersistedDir {
                private_path: "sessions".to_owned(),
                cache_path: "codex/sessions".to_owned(),
            }],
            skills_dir: Some("skills".to_owned()),
            env: BTreeMap::new(),
        };
        if let Some(schema) = context.settings.output_schema.as_ref() {
            setup.files.insert(
                CODEX_SCHEMA_FILENAME.to_owned(),
                output_schema_file(schema)?,
            );
        }
        setup.env.extend(
            options
                .env
                .iter()
                .map(|(key, value)| (key.clone(), SecretString::from(value.clone()))),
        );
        setup
            .env
            .insert(CODEX_TOKEN_ENV.to_owned(), gateway.api_key.clone());
        setup.env.insert(
            CODEX_HOME_ENV.to_owned(),
            SecretString::from(private_dir.to_owned()),
        );
        Ok(setup)
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
        let mut argv = vec!["codex".to_owned(), "exec".to_owned()];
        if let Some(id) = turn.native_session_id {
            argv.extend(["resume".to_owned(), id.to_owned()]);
        }
        argv.extend(["--json".to_owned(), "--skip-git-repo-check".to_owned()]);
        if let Some(model) = context.settings.model.as_ref() {
            argv.extend(["-m".to_owned(), model.clone()]);
        }
        argv.extend(config::pairs("-c", &provider_overrides(context)?));
        argv.extend(permission_args(
            context.settings.permissions,
            context.sandbox.is_container(),
            turn.native_session_id,
        ));
        argv.extend(config::pairs("-c", &feature_overrides(context, &options)));
        argv.extend(config::pairs("-c", &config_overrides(&options.config)?));
        if turn.setup.files.contains_key(CODEX_SCHEMA_FILENAME) {
            argv.extend([
                "--output-schema".to_owned(),
                format!("{}/{}", turn.private_dir, CODEX_SCHEMA_FILENAME),
            ]);
        }
        if turn.native_session_id.is_none() {
            argv.extend(["-C".to_owned(), context.sandbox.workdir().to_owned()]);
        }
        argv.push("-".to_owned());
        Ok(HarnessTurnRequest {
            argv,
            env: turn.setup.env.clone(),
            stdin: turn.prompt.to_owned(),
            cwd: Some(context.sandbox.workdir().to_owned()),
        })
    }
    fn create_stream_state(&self) -> Self::StreamState {
        CodexStreamState::default()
    }
    fn transform_stream_line(
        &self,
        line: &JsonObject,
        state: &mut Self::StreamState,
    ) -> Vec<Event> {
        let Some(event) = stream::parse(line) else {
            return Vec::new();
        };
        match event {
            StreamEvent::ThreadStarted { thread_id } => {
                record_thread_id(thread_id, state);
                Vec::new()
            }
            StreamEvent::ItemStarted { item } => {
                transform_item_event(item, ItemEventKind::Started, state)
            }
            StreamEvent::ItemUpdated { item } => {
                transform_item_event(item, ItemEventKind::Updated, state)
            }
            StreamEvent::ItemCompleted { item } => {
                transform_item_event(item, ItemEventKind::Completed, state)
            }
            StreamEvent::Error { message } => {
                state.error = Some(
                    message
                        .filter(|text| !text.is_empty())
                        .unwrap_or_else(|| "codex reported an error".to_owned()),
                );
                Vec::new()
            }
            StreamEvent::TurnFailed { error } => {
                record_turn_failed(error, state);
                Vec::new()
            }
            StreamEvent::Other => Vec::new(),
        }
    }
    fn get_native_session_id(&self, state: &Self::StreamState) -> Option<String> {
        state.thread_id.clone()
    }
    fn transform_turn_response<S>(
        &self,
        context: &SessionContext<S>,
        state: &Self::StreamState,
        exit_code: i32,
        stderr_tail: &[String],
    ) -> Result<HarnessTurnResponse, HarnessError> {
        if state.failed {
            return Err(HarnessError::TurnFailed {
                reason: state
                    .error
                    .clone()
                    .unwrap_or_else(|| "codex turn failed".to_owned()),
            });
        }
        if exit_code != 0_i32 {
            return Err(HarnessError::TurnFailed {
                reason: format!("codex exited with code {exit_code}: {}", {
                    let tail = stderr_tail_text(stderr_tail);
                    if tail.is_empty() {
                        state
                            .error
                            .clone()
                            .unwrap_or_else(|| "no output".to_owned())
                    } else {
                        tail
                    }
                }),
            });
        }
        Ok(HarnessTurnResponse {
            final_text: state.final_text.clone(),
            output_json: context
                .settings
                .output_schema
                .as_ref()
                .map(|_| state.final_text.clone()),
        })
    }
}

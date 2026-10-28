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
use crate::types::JsonObject;
#[path = "capabilities.rs"]
mod capabilities;
#[path = "config.rs"]
mod config;
#[path = "features.rs"]
mod features;
#[path = "provider.rs"]
mod provider;
#[path = "stream.rs"]
pub mod stream;
mod tests;
use config::config_overrides;
use config::{
    CODEX_HOME_ENV, CODEX_SCHEMA_FILENAME, CODEX_TOKEN_ENV, mismatch, pairs, permission_args,
};
use features::feature_overrides;
use provider::provider_overrides;
use stream::CodexStreamState;

#[derive(Clone, Copy, Debug, Default)]
pub struct CodexHarnessConfig;

fn agent_message_events(item: &JsonObject, state: &mut CodexStreamState) -> Vec<Event> {
    let text = stream::item_text(item).unwrap_or_default().to_owned();
    state.final_text.clone_from(&text);
    if text.is_empty() {
        return Vec::new();
    }
    vec![Event::Text { delta: text }]
}

fn reasoning_events(item: &JsonObject) -> Vec<Event> {
    let Some(text) = stream::item_text(item).filter(|text| !text.is_empty()) else {
        return Vec::new();
    };
    vec![Event::Reasoning {
        delta: text.to_owned(),
    }]
}

fn tool_events(item: &JsonObject, event_type: &str, state: &mut CodexStreamState) -> Vec<Event> {
    let id = stream::item_id(item);
    let mut events = Vec::new();
    if state.started.insert(id.clone()) {
        let (name, native_name, input, builtin) = stream::tool_input(item);
        events.push(Event::ToolCall {
            id: id.clone(),
            name,
            native_name,
            input,
            builtin,
        });
    }
    if event_type == "item.completed" {
        let (output, is_error) = stream::tool_output(item);
        events.push(Event::ToolResult {
            id,
            output,
            is_error,
        });
    }
    events
}

fn transform_item_event(
    line: &JsonObject,
    event_type: &str,
    state: &mut CodexStreamState,
) -> Vec<Event> {
    let Some(item) = stream::object(line.get("item")) else {
        return Vec::new();
    };
    match stream::item_kind(&item) {
        "agent_message" if event_type == "item.completed" => agent_message_events(&item, state),
        "reasoning" if event_type == "item.completed" => reasoning_events(&item),
        "command_execution" | "file_change" | "web_search" | "mcp_tool_call" => {
            tool_events(&item, event_type, state)
        }
        _ => Vec::new(),
    }
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
        let gateway = context.settings.gateway.as_ref().ok_or_else(mismatch)?;
        let options = self.get_options(context)?;
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
            let strict = strict_json_schema(config::schema_value(schema), 0)
                .map_err(|_schema_err| mismatch())?;
            setup.files.insert(
                CODEX_SCHEMA_FILENAME.to_owned(),
                serde_json::to_vec(&strict).map_err(|_encode_err| mismatch())?,
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
        argv.extend(pairs("-c", &provider_overrides(context)?));
        argv.extend(permission_args(context, turn.native_session_id.is_some()));
        argv.extend(pairs("-c", &feature_overrides(context, &options)));
        argv.extend(pairs("-c", &config_overrides(&options.config)?));
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
        match stream::event_type(line) {
            Some("thread.started") => {
                if let Some(thread_id) = stream::thread_id(line) {
                    state.thread_id = Some(thread_id.to_owned());
                }
                Vec::new()
            }
            Some("error") => {
                state.error = Some(
                    stream::error_message(line)
                        .unwrap_or("codex reported an error")
                        .to_owned(),
                );
                Vec::new()
            }
            Some("turn.failed") => {
                state.failed = true;
                state.error = Some(
                    stream::turn_failed_message(line)
                        .or(state.error.as_deref())
                        .unwrap_or("codex turn failed")
                        .to_owned(),
                );
                Vec::new()
            }
            Some(kind @ ("item.started" | "item.updated" | "item.completed")) => {
                transform_item_event(line, kind, state)
            }
            _ => Vec::new(),
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

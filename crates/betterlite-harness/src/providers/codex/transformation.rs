#![expect(
    clippy::complexity,
    clippy::nursery,
    clippy::pedantic,
    clippy::restriction,
    clippy::style,
    reason = "This adapter mirrors a third-party CLI protocol within the existing harness contract"
)]

use std::collections::{BTreeMap, BTreeSet};

use secrecy::SecretString;
use serde_json::Value;

use crate::context::SessionContext;
use crate::errors::HarnessError;
use crate::events::Event;
use crate::options::{CodexOptions, OptionsKind, ReasoningEffort};
use crate::payloads::{stderr_tail_text, strict_json_schema};
use crate::sandbox::Sandbox;
use crate::transformation::{
    BaseCLIHarnessConfig, BaseHarnessConfig, HarnessSessionSetup, HarnessTurnRequest,
    HarnessTurnResponse, PersistedDir, TurnInput,
};
use crate::types::{Capabilities, Capability, Harness, JsonObject, PermissionMode};

pub const CODEX_TOKEN_ENV: &str = "LITELLM_HARNESS_TOKEN";
pub const CODEX_SCHEMA_FILENAME: &str = "output_schema.json";
const CODEX_HOME_ENV: &str = "CODEX_HOME";
const MANAGED_CONFIG_KEYS: [&str; 8] = [
    "model",
    "model_provider",
    "model_providers",
    "approval_policy",
    "sandbox_mode",
    "mcp_servers",
    "developer_instructions",
    "web_search",
];

#[derive(Clone, Debug, Default)]
pub struct CodexStreamState {
    pub thread_id: Option<String>,
    pub final_text: String,
    pub error: Option<String>,
    pub failed: bool,
    started: BTreeSet<String>,
}

fn mismatch() -> HarnessError {
    HarnessError::OptionsMismatch {
        given: "CodexOptions.config",
        harness: Harness::Codex,
        expected: "valid Codex configuration",
    }
}

pub fn toml_key(key: &str) -> String {
    if key
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        key.to_owned()
    } else {
        serde_json::to_string(key).unwrap_or_else(|_| "\"\"".to_owned())
    }
}

pub fn toml_value(value: &Value, depth: usize) -> Result<String, HarnessError> {
    if depth > crate::payloads::DEFAULT_MAX_RECURSE_DEPTH {
        return Err(mismatch());
    }
    match value {
        Value::Bool(value) => Ok(value.to_string()),
        Value::Number(value) => Ok(value.to_string()),
        Value::String(value) => serde_json::to_string(value).map_err(|_| mismatch()),
        Value::Array(values) => values
            .iter()
            .map(|value| toml_value(value, depth.saturating_add(1)))
            .collect::<Result<Vec<_>, _>>()
            .map(|values| format!("[{}]", values.join(", "))),
        Value::Object(values) => values
            .iter()
            .map(|(key, value)| {
                Ok(format!(
                    "{} = {}",
                    toml_key(key),
                    toml_value(value, depth.saturating_add(1))?
                ))
            })
            .collect::<Result<Vec<_>, HarnessError>>()
            .map(|values| format!("{{{}}}", values.join(", "))),
        Value::Null => Err(mismatch()),
    }
}

pub fn config_overrides(config: &JsonObject) -> Result<Vec<String>, HarnessError> {
    config
        .iter()
        .map(|(key, value)| {
            let first = key.split('.').next().unwrap_or_default();
            if key.is_empty() || key.contains('=') || MANAGED_CONFIG_KEYS.contains(&first) {
                return Err(mismatch());
            }
            Ok(format!("{}={}", key, toml_value(value, 0)?))
        })
        .collect()
}

fn pairs(flag: &str, values: &[String]) -> Vec<String> {
    values
        .iter()
        .flat_map(|value| [flag.to_owned(), value.clone()])
        .collect()
}

fn provider_overrides<S>(context: &SessionContext<S>) -> Result<Vec<String>, HarnessError> {
    let gateway = context.settings.gateway.as_ref().ok_or_else(mismatch)?;
    let base = format!("{}/v1", gateway.api_base.trim_end_matches('/'));
    Ok(vec![
        "model_provider=litellm".to_owned(),
        "model_providers.litellm.name=litellm".to_owned(),
        format!(
            "model_providers.litellm.base_url={}",
            toml_value(&Value::String(base), 0)?
        ),
        format!("model_providers.litellm.env_key={CODEX_TOKEN_ENV}"),
        "model_providers.litellm.wire_api=responses".to_owned(),
        "approval_policy=never".to_owned(),
    ])
}

fn permission_args<S: Sandbox>(context: &SessionContext<S>, resumed: bool) -> Vec<String> {
    if context.settings.permissions != PermissionMode::ReadOnly && context.sandbox.is_container() {
        return vec!["--dangerously-bypass-approvals-and-sandbox".to_owned()];
    }
    let mode = match context.settings.permissions {
        PermissionMode::ReadOnly => "read-only",
        PermissionMode::Full | PermissionMode::Ask | PermissionMode::Edit => "workspace-write",
    };
    if resumed {
        vec!["-c".to_owned(), format!("sandbox_mode=\"{mode}\"")]
    } else {
        vec!["--sandbox".to_owned(), mode.to_owned()]
    }
}

fn feature_overrides<S>(context: &SessionContext<S>, options: &CodexOptions) -> Vec<String> {
    let mut values = vec![format!(
        "web_search={}",
        if options.web_search {
            "live"
        } else {
            "disabled"
        }
    )];
    if let Some(effort) = options.reasoning_effort {
        let value = match effort {
            ReasoningEffort::Low => "low",
            ReasoningEffort::Medium => "medium",
            ReasoningEffort::High => "high",
            ReasoningEffort::Xhigh => "xhigh",
        };
        values.push(format!("model_reasoning_effort={value}"));
        values.push("model_reasoning_summary=auto".to_owned());
    }
    if let Some(instructions) = context.settings.instructions.as_ref() {
        if let Ok(encoded) = toml_value(&Value::String(instructions.clone()), 0) {
            values.push(format!("developer_instructions={encoded}"));
        }
    }
    values
}

fn object(value: Option<&Value>) -> Option<JsonObject> {
    value.and_then(Value::as_object).map(|value| {
        value
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect()
    })
}

fn tool_input(item: &JsonObject) -> (String, String, JsonObject, bool) {
    let kind = item.get("type").and_then(Value::as_str).unwrap_or_default();
    match kind {
        "command_execution" => (
            "bash".to_owned(),
            kind.to_owned(),
            BTreeMap::from([(
                "command".to_owned(),
                item.get("command")
                    .cloned()
                    .unwrap_or(Value::String(String::new())),
            )]),
            true,
        ),
        "file_change" => (
            "edit".to_owned(),
            "apply_patch".to_owned(),
            BTreeMap::from([(
                "changes".to_owned(),
                item.get("changes")
                    .cloned()
                    .unwrap_or(Value::Array(Vec::new())),
            )]),
            true,
        ),
        "web_search" => (
            "web_search".to_owned(),
            kind.to_owned(),
            BTreeMap::from([(
                "query".to_owned(),
                item.get("query")
                    .cloned()
                    .unwrap_or(Value::String(String::new())),
            )]),
            true,
        ),
        _ => {
            let server = item
                .get("server")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let native = item.get("tool").and_then(Value::as_str).unwrap_or_default();
            let input = match item.get("arguments") {
                Some(Value::Object(map)) => {
                    map.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
                }
                Some(value) => BTreeMap::from([("arguments".to_owned(), value.clone())]),
                None => BTreeMap::new(),
            };
            (
                if server.is_empty() {
                    native.to_owned()
                } else {
                    format!("{server}.{native}")
                },
                native.to_owned(),
                input,
                false,
            )
        }
    }
}

fn tool_output(item: &JsonObject) -> (String, bool) {
    let kind = item.get("type").and_then(Value::as_str).unwrap_or_default();
    let status = item.get("status").and_then(Value::as_str);
    match kind {
        "command_execution" => (
            item.get("aggregated_output")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            status == Some("failed")
                || item
                    .get("exit_code")
                    .is_some_and(|v| v.as_i64().is_some_and(|code| code != 0)),
        ),
        "file_change" => (
            item.get("changes")
                .and_then(Value::as_array)
                .map(|changes| {
                    changes
                        .iter()
                        .map(|change| {
                            format!(
                                "{} {}",
                                change
                                    .get("kind")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default(),
                                change
                                    .get("path")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                            )
                            .trim()
                            .to_owned()
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default(),
            status == Some("failed"),
        ),
        "web_search" => (String::new(), status == Some("failed")),
        _ => match item.get("error") {
            Some(error) => (
                error
                    .get("message")
                    .and_then(Value::as_str)
                    .map_or_else(|| error.to_string(), str::to_owned),
                true,
            ),
            None => (
                item.get("result").map_or_else(String::new, |value| {
                    value
                        .as_str()
                        .map_or_else(|| value.to_string(), str::to_owned)
                }),
                status == Some("failed"),
            ),
        },
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct CodexHarnessConfig;

impl BaseHarnessConfig for CodexHarnessConfig {
    type Options = CodexOptions;
    const HARNESS: Harness = Harness::Codex;
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            supported: BTreeSet::from([
                Capability::StructuredOutput,
                Capability::Skills,
                Capability::Resume,
            ]),
            permission_modes: BTreeSet::from([PermissionMode::ReadOnly, PermissionMode::Full]),
        }
    }
    fn validate_environment<S>(&self, context: &SessionContext<S>) -> Result<(), HarnessError> {
        let options = self.get_options(context)?;
        config_overrides(&options.config).map(|_| ())
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
        let mut setup = HarnessSessionSetup {
            persisted_dirs: vec![PersistedDir {
                private_path: "sessions".to_owned(),
                cache_path: "codex/sessions".to_owned(),
            }],
            skills_dir: Some("skills".to_owned()),
            ..HarnessSessionSetup::default()
        };
        if let Some(schema) = context.settings.output_schema.as_ref() {
            let strict = strict_json_schema(
                Value::Object(schema.iter().map(|(k, v)| (k.clone(), v.clone())).collect()),
                0,
            )
            .map_err(|_| mismatch())?;
            setup.files.insert(
                CODEX_SCHEMA_FILENAME.to_owned(),
                serde_json::to_vec(&strict).map_err(|_| mismatch())?,
            );
        }
        setup.env.extend(
            context
                .settings
                .options
                .as_ref()
                .and_then(|o| CodexOptions::pick(o))
                .map_or_else(BTreeMap::new, |o| {
                    o.env
                        .iter()
                        .map(|(k, v)| (k.clone(), SecretString::from(v.clone())))
                        .collect()
                }),
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
        match line.get("type").and_then(Value::as_str) {
            Some("thread.started") => {
                state.thread_id = line
                    .get("thread_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                Vec::new()
            }
            Some("error") => {
                state.error = Some(
                    line.get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("codex reported an error")
                        .to_owned(),
                );
                Vec::new()
            }
            Some("turn.failed") => {
                state.failed = true;
                state.error = line
                    .get("error")
                    .and_then(Value::as_object)
                    .and_then(|e| e.get("message"))
                    .and_then(Value::as_str)
                    .or_else(|| line.get("error").and_then(Value::as_str))
                    .or(state.error.as_deref())
                    .map_or_else(|| "codex turn failed".to_owned(), str::to_owned)
                    .into();
                Vec::new()
            }
            Some(event_type @ ("item.started" | "item.updated" | "item.completed")) => {
                let Some(item) = object(line.get("item")) else {
                    return Vec::new();
                };
                let kind = item.get("type").and_then(Value::as_str).unwrap_or_default();
                let id = item
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                if kind == "agent_message" && event_type == "item.completed" {
                    let text = item
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned();
                    state.final_text = text.clone();
                    return if text.is_empty() {
                        Vec::new()
                    } else {
                        vec![Event::Text { delta: text }]
                    };
                }
                if kind == "reasoning" && event_type == "item.completed" {
                    return item
                        .get("text")
                        .and_then(Value::as_str)
                        .filter(|text| !text.is_empty())
                        .map_or_else(Vec::new, |text| {
                            vec![Event::Reasoning {
                                delta: text.to_owned(),
                            }]
                        });
                }
                if ![
                    "command_execution",
                    "file_change",
                    "web_search",
                    "mcp_tool_call",
                ]
                .contains(&kind)
                {
                    return Vec::new();
                }
                let mut events = Vec::new();
                if state.started.insert(id.clone()) {
                    let (name, native_name, input, builtin) = tool_input(&item);
                    events.push(Event::ToolCall {
                        id: id.clone(),
                        name,
                        native_name,
                        input,
                        builtin,
                    });
                }
                if event_type == "item.completed" {
                    let (output, is_error) = tool_output(&item);
                    events.push(Event::ToolResult {
                        id,
                        output,
                        is_error,
                    });
                }
                events
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
        if exit_code != 0 {
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

#[cfg(test)]
mod tests {
    use super::{CodexHarnessConfig, CodexStreamState, config_overrides, toml_key, toml_value};
    use crate::events::Event;
    use crate::transformation::BaseCLIHarnessConfig;
    use crate::types::JsonObject;
    use serde_json::json;

    #[test]
    fn toml_values_and_managed_keys_match_codex_syntax() {
        assert_eq!(toml_key("plain-key"), "plain-key");
        assert_eq!(toml_key("a key"), "\"a key\"");
        assert_eq!(
            toml_value(&json!({"a key": [true, 2]}), 0).unwrap(),
            "{\"a key\" = [true, 2]}"
        );
        let config = JsonObject::from([("model".to_owned(), json!("wrong"))]);
        assert!(config_overrides(&config).is_err());
    }

    #[test]
    fn stream_parser_orders_tool_call_before_result() {
        let mut state = CodexStreamState::default();
        let line: JsonObject = serde_json::from_value(json!({
            "type": "item.completed",
            "item": {"id": "tool-1", "type": "command_execution", "command": "echo hi", "aggregated_output": "hi\\n", "exit_code": 0, "status": "completed"}
        })).unwrap();
        let events = CodexHarnessConfig.transform_stream_line(&line, &mut state);
        assert!(matches!(events.first(), Some(Event::ToolCall { .. })));
        assert!(matches!(
            events.get(1),
            Some(Event::ToolResult {
                is_error: false,
                ..
            })
        ));
    }
}

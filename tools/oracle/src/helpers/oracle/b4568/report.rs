use super::Setup;
use super::fakes::{Launcher, Recorder};
use crate::helpers::HelperError;
use betterlite_cli::agent_launch::EnvVars;
use betterlite_cli::agent_run::error::AgentRunError;
use serde_json::{Map, Value, json};
use std::fmt;
use std::fs;
use std::path::Path;
use std::sync::PoisonError;
pub(super) fn report(
    result: &Result<i32, AgentRunError>,
    launcher: &Launcher,
    messages: &Recorder,
    setup: &Setup,
) -> Value {
    let root = setup.root.as_str();
    json!({
        "error": match result { Ok(_code) => Value::Null, Err(error) => error_json(error) },
        "warnings": messages.warns.iter().map(|value| value.replace(root, "<tmp>")).collect::<Vec<String>>(),
        "notices": messages.notices.iter().map(|value| value.replace(root, "<tmp>")).collect::<Vec<String>>(),
        "launch": launch_json(launcher, root),
        "catalog": file_json(&setup.directory.path().join("codex/litellm-models.json")),
        "pi_models": file_json(&setup.directory.path().join("pi/models.json")),
    })
}
pub(super) fn failure<M>(message: M) -> Box<HelperError>
where
    M: fmt::Display,
{
    Box::new(HelperError {
        class: "AgentExecution".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_string(),
    })
}
pub(super) fn path_text(path: &Path) -> Result<String, Box<HelperError>> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| failure("a scripted path is not valid UTF-8"))
}
fn error_json(error: &AgentRunError) -> Value {
    if let AgentRunError::MissingBinary { hint, .. } = error {
        return json!({
            "kind": "missing_binary",
            "docs": hint.strip_prefix(" Install it first: "),
        });
    }
    json!({"kind": "failed", "docs": null})
}
fn env_json(env: &EnvVars, root: &str) -> Value {
    let mut fields = Map::new();
    for (name, value) in env {
        let text = value.replace(root, "<tmp>");
        let entry = if name == "OPENCODE_CONFIG_CONTENT" {
            serde_json::from_str(text.as_str()).unwrap_or(Value::String(text))
        } else {
            Value::String(text)
        };
        fields.insert(name.clone(), entry);
    }
    Value::Object(fields)
}
fn launch_json(launcher: &Launcher, root: &str) -> Value {
    let launch = launcher
        .calls
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .first()
        .cloned();
    let Some(call) = launch else {
        return Value::Null;
    };
    json!({
        "binary": call.path.replace(root, "<tmp>"),
        "argv": call.args.iter().map(|argument| argument.replace(root, "<tmp>")).collect::<Vec<String>>(),
        "env": env_json(&call.env, root),
    })
}
fn file_json(path: &Path) -> Value {
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .unwrap_or(Value::Null)
}

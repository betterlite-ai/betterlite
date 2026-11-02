use std::env::consts::{ARCH, OS};

use betterlite_config::sections::chatgpt_headers::Settings as HeaderSettings;
use nix::sys::utsname::uname;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::openai_base_config::Headers;

pub const DEFAULT_ORIGINATOR: &str = "codex_cli_rs";
pub const DEFAULT_USER_AGENT: &str = "codex_cli_rs/0.0.0 (Unknown 0; unknown) unknown";
const COMPATIBILITY_VERSION: &str = "1.105.1";
const CHATGPT_DEFAULT_INSTRUCTIONS: &str = include_str!("default_instructions.txt");

pub type ParamsRecord = Map<String, Value>;

// reads the real OS, kernel release and architecture so unit tests can inject deterministic values
pub trait PlatformEnvironment: Send + Sync {
    fn os_type(&self) -> String;
    fn os_release(&self) -> String;
    fn arch(&self) -> String;
}

#[derive(Debug, Default)]
pub struct SystemPlatformEnvironment;

impl PlatformEnvironment for SystemPlatformEnvironment {
    fn os_type(&self) -> String {
        match OS {
            "macos" => "Darwin",
            "linux" => "Linux",
            "windows" => "Windows",
            other => other,
        }
        .to_owned()
    }

    // mirrors Python's platform.release(), which reads the same uname(2) release field
    fn os_release(&self) -> String {
        let Ok(info) = uname() else {
            return "0".to_owned();
        };
        let Some(release) = info.release().to_str() else {
            return "0".to_owned();
        };
        if release.is_empty() {
            "0".to_owned()
        } else {
            release.to_owned()
        }
    }

    fn arch(&self) -> String {
        ARCH.to_owned()
    }
}

pub trait SessionIdentity: Send + Sync {
    fn generate(&self) -> String;
}

#[derive(Debug, Default)]
pub struct UuidSessionIdentity;

impl SessionIdentity for UuidSessionIdentity {
    fn generate(&self) -> String {
        Uuid::new_v4().to_string()
    }
}

// mirrors Python's duck-typed litellm_params: absent, a mapping, or an object whose model_dump/dict may be missing or may raise
#[derive(Debug, Clone)]
pub enum LitellmParamsInput {
    Absent,
    Mapping(ParamsRecord),
    ModelSnapshot {
        model_dump: ModelDumpOutcome,
        dict: ModelDumpOutcome,
    },
}

#[derive(Debug, Clone, Default)]
pub enum ModelDumpOutcome {
    #[default]
    Unavailable,
    Succeeded(ParamsRecord),
    Failed,
}

fn normalize_litellm_params(input: &LitellmParamsInput) -> ParamsRecord {
    match input {
        LitellmParamsInput::Absent => ParamsRecord::new(),
        LitellmParamsInput::Mapping(fields) => fields.clone(),
        // a failing model_dump never falls through to dict(), even when dict() would succeed
        LitellmParamsInput::ModelSnapshot { model_dump, dict } => match model_dump {
            ModelDumpOutcome::Succeeded(fields) => fields.clone(),
            ModelDumpOutcome::Failed => ParamsRecord::new(),
            ModelDumpOutcome::Unavailable => match dict {
                ModelDumpOutcome::Succeeded(fields) => fields.clone(),
                ModelDumpOutcome::Failed | ModelDumpOutcome::Unavailable => ParamsRecord::new(),
            },
        },
    }
}

fn safe_header_value(value: &str) -> String {
    if value.is_empty() {
        return String::new();
    }
    value
        .chars()
        .map(|letter| {
            if (32..=126).contains(&u32::from(letter)) {
                letter
            } else {
                '_'
            }
        })
        .collect()
}

#[must_use]
pub fn get_chatgpt_originator(settings: &HeaderSettings) -> String {
    let originator = settings
        .originator_override
        .as_deref()
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_ORIGINATOR);
    let safe = safe_header_value(originator);
    if safe.is_empty() {
        DEFAULT_ORIGINATOR.to_owned()
    } else {
        safe
    }
}

#[must_use]
pub fn get_chatgpt_user_agent(
    settings: &HeaderSettings,
    platform: &dyn PlatformEnvironment,
    originator: &str,
) -> String {
    if let Some(override_value) = settings
        .user_agent_override
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        let safe = safe_header_value(override_value);
        return if safe.is_empty() {
            DEFAULT_USER_AGENT.to_owned()
        } else {
            safe
        };
    }
    let trimmed_suffix = settings.user_agent_suffix.as_deref().unwrap_or("").trim();
    let suffix = if trimmed_suffix.is_empty() {
        String::new()
    } else {
        format!(" ({trimmed_suffix})")
    };
    let candidate = format!(
        "{originator}/{COMPATIBILITY_VERSION} ({} {}; {}) {}{suffix}",
        platform.os_type(),
        platform.os_release(),
        platform.arch(),
        settings.terminal,
    );
    let safe = safe_header_value(&candidate);
    if safe.is_empty() {
        DEFAULT_USER_AGENT.to_owned()
    } else {
        safe
    }
}

#[must_use]
pub fn get_chatgpt_default_headers(
    settings: &HeaderSettings,
    platform: &dyn PlatformEnvironment,
    access_token: &SecretString,
    account_id: Option<&str>,
    session_id: Option<&str>,
) -> Headers {
    let originator = get_chatgpt_originator(settings);
    let user_agent = get_chatgpt_user_agent(settings, platform, &originator);
    let mut headers = Headers::new();
    headers.insert(
        "Authorization".to_owned(),
        format!("Bearer {}", access_token.expose_secret()),
    );
    headers.insert("content-type".to_owned(), "application/json".to_owned());
    headers.insert("accept".to_owned(), "text/event-stream".to_owned());
    headers.insert("originator".to_owned(), originator);
    headers.insert("user-agent".to_owned(), user_agent);
    if let Some(truthy_session_id) = session_id.filter(|value| !value.is_empty()) {
        headers.insert("session_id".to_owned(), truthy_session_id.to_owned());
    }
    if let Some(truthy_account_id) = account_id.filter(|value| !value.is_empty()) {
        headers.insert(
            "ChatGPT-Account-Id".to_owned(),
            truthy_account_id.to_owned(),
        );
    }
    headers
}

#[must_use]
pub fn get_chatgpt_instructions(settings: &HeaderSettings) -> &str {
    settings
        .default_instructions_override
        .as_deref()
        .filter(|value| !value.is_empty())
        .unwrap_or(CHATGPT_DEFAULT_INSTRUCTIONS)
}

#[must_use]
pub fn get_chatgpt_session_id(litellm_params: &LitellmParamsInput) -> Option<String> {
    let params = normalize_litellm_params(litellm_params);
    for key in ["litellm_session_id", "session_id"] {
        if let Some(value) = params.get(key).and_then(present_session_text) {
            return Some(value);
        }
    }
    if let Some(Value::Object(metadata)) = params.get("metadata")
        && let Some(value) = metadata.get("session_id").and_then(present_session_text)
    {
        return Some(value);
    }
    for key in ["litellm_trace_id", "litellm_call_id"] {
        if let Some(value) = params.get(key).and_then(present_session_text) {
            return Some(value);
        }
    }
    None
}

#[must_use]
pub fn ensure_chatgpt_session_id(
    litellm_params: &LitellmParamsInput,
    identity: &dyn SessionIdentity,
) -> String {
    let Some(session_id) = get_chatgpt_session_id(litellm_params) else {
        return identity.generate();
    };
    session_id
}

fn present_session_text(value: &Value) -> Option<String> {
    if !has_session_value(value) {
        return None;
    }
    Some(match value {
        Value::String(text) => text.clone(),
        other @ (Value::Null
        | Value::Bool(_)
        | Value::Number(_)
        | Value::Array(_)
        | Value::Object(_)) => stringify_value(other),
    })
}

fn has_session_value(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64().is_none_or(|float| float != 0.0_f64),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(fields) => !fields.is_empty(),
    }
}

// mirrors Python's str(): bare for strings, recursing into containers via repr for their elements
fn stringify_value(value: &Value) -> String {
    match value {
        Value::Null => "None".to_owned(),
        Value::Bool(true) => "True".to_owned(),
        Value::Bool(false) => "False".to_owned(),
        Value::Number(number) => number.to_string(),
        Value::String(text) => text.clone(),
        Value::Array(items) => {
            let rendered: Vec<String> = items.iter().map(quoted_value).collect();
            format!("[{}]", rendered.join(", "))
        }
        Value::Object(fields) => {
            let rendered: Vec<String> = fields
                .iter()
                .map(|(key, item)| {
                    format!(
                        "{}: {}",
                        quoted_value(&Value::String(key.clone())),
                        quoted_value(item)
                    )
                })
                .collect();
            format!("{{{}}}", rendered.join(", "))
        }
    }
}

fn quoted_value(value: &Value) -> String {
    match value {
        Value::String(text) => format!("'{}'", text.replace('\\', "\\\\").replace('\'', "\\'")),
        other @ (Value::Null
        | Value::Bool(_)
        | Value::Number(_)
        | Value::Array(_)
        | Value::Object(_)) => stringify_value(other),
    }
}

mod tests;

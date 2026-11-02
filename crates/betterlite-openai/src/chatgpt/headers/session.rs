use serde_json::{Map, Value};
use uuid::Uuid;

pub type ParamsRecord = Map<String, Value>;

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
    if let Value::String(text) = value {
        format!("'{}'", text.replace('\\', "\\\\").replace('\'', "\\'"))
    } else {
        stringify_value(value)
    }
}

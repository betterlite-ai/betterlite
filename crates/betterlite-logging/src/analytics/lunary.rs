use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::value::{Map, Value};
use thiserror::Error;

pub trait LunarySink {
    fn track_event(&mut self, event: LunaryEvent);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LunaryEvent {
    pub event_type: String,
    pub event: String,
    pub run_id: String,
    pub timestamp: String,
    pub runtime: String,
    pub parent_run_id: Option<String>,
    pub user_id: Option<String>,
    pub name: Option<String>,
    pub input: Option<Value>,
    pub template_id: Option<String>,
    pub metadata: Value,
    pub tags: Option<Value>,
    pub params: Value,
    pub error: Option<Value>,
    pub output: Option<Value>,
    pub token_usage: Option<Value>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LunaryError {
    #[error("Lunary version is invalid: {version}")]
    InvalidVersion { version: String },
    #[error("Lunary version {version} is outdated; required >= 0.1.43")]
    OutdatedVersion { version: String },
}

pub struct LunaryLogger<Client> {
    client: Client,
}

impl<Client> LunaryLogger<Client> {
    pub fn new(client: Client, version: &str) -> Result<Self, LunaryError> {
        let parsed = parse_version(version).ok_or_else(|| LunaryError::InvalidVersion {
            version: version.to_owned(),
        })?;
        if parsed < (0, 1, 43) {
            return Err(LunaryError::OutdatedVersion {
                version: version.to_owned(),
            });
        }
        Ok(Self { client })
    }
}

impl<Client: LunarySink> LunaryLogger<Client> {
    pub fn log_event(
        &mut self,
        request: &LunaryRequest,
        response: Option<&Value>,
        terminal_event: &str,
        error: Option<&str>,
    ) {
        let metadata = request
            .metadata
            .clone()
            .unwrap_or_else(|| Value::Object(Map::new()));
        self.client.track_event(start_event(request, &metadata));
        let (output, token_usage) = response.map_or((None, None), |value| {
            let output = value.get("choices").cloned();
            let usage = value.get("usage").map(parse_usage);
            (output.and_then(|item| parse_messages(&item)), usage)
        });
        let terminal = end_event(request, terminal_event, error, output, token_usage);
        self.client.track_event(terminal);
    }

    pub fn into_client(self) -> Client {
        self.client
    }
}

fn start_event(request: &LunaryRequest, metadata: &Value) -> LunaryEvent {
    LunaryEvent {
        event_type: request.event_type.clone(),
        event: "start".to_owned(),
        run_id: request.run_id.clone(),
        timestamp: format_time(request.start_time),
        runtime: "litellm".to_owned(),
        parent_run_id: string_field(metadata, "parent_run_id"),
        user_id: request.user_id.clone(),
        name: Some(request.model.clone()),
        input: parse_messages(request.input.as_ref().unwrap_or(&Value::Null)),
        template_id: request.template_id.clone(),
        metadata: metadata.clone(),
        tags: metadata.get("tags").cloned(),
        params: request.params.clone(),
        error: None,
        output: None,
        token_usage: None,
    }
}

fn end_event(
    request: &LunaryRequest,
    event: &str,
    error: Option<&str>,
    output: Option<Value>,
    usage: Option<Value>,
) -> LunaryEvent {
    let error_value = error.map(|message| {
        let mut object = Map::new();
        object.insert("stack".to_owned(), Value::String(message.to_owned()));
        Value::Object(object)
    });
    LunaryEvent {
        event_type: request.event_type.clone(),
        event: event.to_owned(),
        run_id: request.run_id.clone(),
        timestamp: format_time(request.end_time),
        runtime: "litellm".to_owned(),
        parent_run_id: None,
        user_id: None,
        name: None,
        input: None,
        template_id: None,
        metadata: Value::Null,
        tags: None,
        params: Value::Object(Map::new()),
        error: error_value,
        output,
        token_usage: usage,
    }
}

pub struct LunaryRequest {
    pub event_type: String,
    pub run_id: String,
    pub model: String,
    pub input: Option<Value>,
    pub metadata: Option<Value>,
    pub params: Value,
    pub template_id: Option<String>,
    pub user_id: Option<String>,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
}

pub fn parse_usage(usage: &Value) -> Value {
    let completion = usage
        .get("completion_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let prompt = usage
        .get("prompt_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let mut parsed = Map::new();
    parsed.insert("completion".to_owned(), Value::from(completion));
    parsed.insert("prompt".to_owned(), Value::from(prompt));
    Value::Object(parsed)
}

pub fn clean_extra_params(extra: &Value) -> (Value, Option<String>) {
    let Some(source) = extra.as_object() else {
        return (Value::Object(Map::new()), None);
    };
    let mut cleaned = Map::new();
    let mut template_id = None;
    for (key, value) in source {
        if key == "extra_body" || key == "user" {
            continue;
        }
        if key == "extra_headers" {
            template_id = value
                .get("Template-Id")
                .and_then(Value::as_str)
                .map(str::to_owned);
            continue;
        }
        let cleaned_value = if key == "tools"
            || value.is_string()
            || value.is_number()
            || value.is_boolean()
            || value.is_null()
        {
            value.clone()
        } else {
            Value::String(value.to_string())
        };
        cleaned.insert(key.clone(), cleaned_value);
    }
    (Value::Object(cleaned), template_id)
}

#[must_use]
pub fn parse_tool_calls(tool_calls: &Value) -> Value {
    let Some(calls) = tool_calls.as_array() else {
        return Value::Null;
    };
    let parsed = calls
        .iter()
        .filter_map(|call| {
            let function = call.get("function");
            let custom = call.get("custom");
            let source = function.or(custom)?;
            let name = source.get("name")?.clone();
            let arguments = source
                .get("arguments")
                .or_else(|| source.get("input"))?
                .clone();
            let mut function_object = Map::new();
            function_object.insert("name".to_owned(), name);
            function_object.insert("arguments".to_owned(), arguments);
            let mut parsed_call = Map::new();
            parsed_call.insert(
                "type".to_owned(),
                call.get("type").cloned().unwrap_or(Value::Null),
            );
            parsed_call.insert(
                "id".to_owned(),
                call.get("id").cloned().unwrap_or(Value::Null),
            );
            parsed_call.insert("function".to_owned(), Value::Object(function_object));
            Some(Value::Object(parsed_call))
        })
        .collect::<Vec<_>>();
    Value::Array(parsed)
}

pub fn parse_messages(input: &Value) -> Option<Value> {
    if input.is_null() {
        return None;
    }
    if let Some(items) = input.as_array() {
        if items.len() == 1 {
            return items.first().and_then(clean_message);
        }
        return Some(Value::Array(
            items.iter().filter_map(clean_message).collect(),
        ));
    }
    clean_message(input)
}

fn clean_message(message: &Value) -> Option<Value> {
    if message.is_string() {
        return Some(message.clone());
    }
    let object = message.as_object()?;
    if let Some(nested) = object.get("message") {
        return clean_message(nested);
    }
    let mut cleaned = Map::new();
    cleaned.insert(
        "role".to_owned(),
        object.get("role").cloned().unwrap_or(Value::Null),
    );
    cleaned.insert(
        "content".to_owned(),
        object.get("content").cloned().unwrap_or(Value::Null),
    );
    if let Some(calls) = object
        .get("tool_calls")
        .filter(|value| !value.is_null() && value.as_array().is_some_and(|items| !items.is_empty()))
    {
        cleaned.insert("tool_calls".to_owned(), parse_tool_calls(calls));
    }
    Some(Value::Object(cleaned))
}

fn parse_version(version: &str) -> Option<(u64, u64, u64)> {
    let mut parts = version.split('.').map(|part| {
        part.split(['-', '+'])
            .next()
            .and_then(|number| number.parse().ok())
    });
    Some((
        parts.next()??,
        parts.next()??,
        parts.next().unwrap_or(Some(0))?,
    ))
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn format_time(time: DateTime<Utc>) -> String {
    time.to_rfc3339_opts(SecondsFormat::Millis, false)
}

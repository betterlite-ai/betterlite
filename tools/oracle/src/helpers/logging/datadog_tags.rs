use std::collections::BTreeMap;

use serde_json::{Value, json};

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Normalize {
        values: Vec<String>,
    },
    Environment {
        env: BTreeMap<String, String>,
    },
    Tags {
        env: BTreeMap<String, String>,
        request_tags: Vec<String>,
        metadata: BTreeMap<String, String>,
    },
    TraceContext {
        current: Option<SpanInput>,
        root: Option<SpanInput>,
    },
    SpanTagging {
        current: Option<SpanInput>,
        root: Option<SpanInput>,
        call_id: Option<String>,
        key_alias: Option<String>,
        key_hash: Option<String>,
        requested_model: Option<String>,
    },
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SpanInput {
    pub(crate) available: bool,
    pub(crate) trace_id: Option<Value>,
    pub(crate) span_id: Option<Value>,
    pub(crate) set_tag_str: Option<bool>,
    pub(crate) set_tag: Option<bool>,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let output = match input {
            Input::Normalize { values } => {
                json!(
                    values
                        .iter()
                        .map(
                            |value| betterlite_logging::datadog::normalize_datadog_tag_value(value)
                        )
                        .collect::<Vec<_>>()
                )
            }
            Input::Environment { env } => environment(env),
            Input::Tags {
                env,
                request_tags,
                metadata,
            } => tags(env, request_tags, metadata),
            Input::TraceContext { current, root } => context(current.as_ref(), root.as_ref()),
            Input::SpanTagging {
                current,
                root,
                call_id,
                key_alias,
                key_hash,
                requested_model,
            } => tagging(
                current.as_ref(),
                root.as_ref(),
                &TagValues {
                    call_id: call_id.as_deref(),
                    key_alias: key_alias.as_deref(),
                    key_hash: key_hash.as_deref(),
                    requested_model: requested_model.as_deref(),
                },
            ),
        };
        Ok(output)
    }
}

fn environment(env: &BTreeMap<String, String>) -> Value {
    let value = |name: &str| env.get(name).map(String::as_str);
    json!({
        "source": value("DD_SOURCE").unwrap_or("litellm"),
        "service": value("DD_SERVICE").unwrap_or("litellm-server"),
        "hostname": value("HOSTNAME").unwrap_or(""),
        "environment": value("DD_ENV").unwrap_or("unknown"),
        "pod_name": value("POD_NAME").unwrap_or("unknown"),
        "version": value("DD_VERSION").unwrap_or("unknown"),
        "base_url": value("DD_BASE_URL"),
    })
}

fn tags(
    env: &BTreeMap<String, String>,
    request_tags: &[String],
    metadata: &BTreeMap<String, String>,
) -> Value {
    let environment = betterlite_logging::datadog::DatadogEnvironment::from_lookup(env);
    let team = [
        "user_api_key_team_alias",
        "team_alias",
        "user_api_key_team_id",
        "team_id",
    ]
    .iter()
    .find_map(|name| metadata.get(*name).map(String::as_str));
    json!(betterlite_logging::datadog::get_datadog_tags(
        &environment,
        request_tags,
        team,
    ))
}

fn context(current: Option<&SpanInput>, root: Option<&SpanInput>) -> Value {
    let active = active_span(current, root);
    let Some(active_span) = active else {
        return Value::Null;
    };
    let Some(trace_id) = active_span.trace_id.as_ref() else {
        return Value::Null;
    };
    let mut result = serde_json::Map::from_iter([("trace_id".to_owned(), as_string(trace_id))]);
    if let Some(span_id) = active_span.span_id.as_ref() {
        result.insert("span_id".to_owned(), as_string(span_id));
    }
    Value::Object(result)
}

fn tagging(current: Option<&SpanInput>, root: Option<&SpanInput>, values: &TagValues<'_>) -> Value {
    let span = active_span(current, root);
    let mut tags = Vec::new();
    for (key, value) in [
        ("litellm.call_id", values.call_id),
        ("litellm.key_alias", values.key_alias),
        ("litellm.key_hash", values.key_hash),
        ("litellm.requested_model", values.requested_model),
    ] {
        if value.is_some_and(|_| {
            span.is_some_and(|active| {
                active.set_tag_str == Some(true) || active.set_tag == Some(true)
            })
        }) {
            tags.push(key);
        }
    }
    json!({"tags": tags, "context": context(current, root)})
}

fn active_span<'span>(
    current: Option<&'span SpanInput>,
    root: Option<&'span SpanInput>,
) -> Option<&'span SpanInput> {
    if let Some(span) = current.filter(|span| span.available) {
        return Some(span);
    }
    if let Some(span) = root.filter(|span| span.available) {
        return Some(span);
    }
    None
}

fn as_string(value: &Value) -> Value {
    match value {
        Value::String(string) => Value::String(string.clone()),
        Value::Null => Value::String(String::from("null")),
        Value::Bool(boolean) => Value::String(boolean.to_string()),
        Value::Number(number) => Value::String(number.to_string()),
        Value::Array(array) => Value::String(Value::Array(array.clone()).to_string()),
        Value::Object(object) => Value::String(Value::Object(object.clone()).to_string()),
    }
}

struct TagValues<'value> {
    call_id: Option<&'value str>,
    key_alias: Option<&'value str>,
    key_hash: Option<&'value str>,
    requested_model: Option<&'value str>,
}

use betterlite_guardrails::cisco_conversation::Fields;
use betterlite_guardrails::cisco_conversation::json::Json;
use betterlite_guardrails::cisco_mcp::blocking;
use betterlite_guardrails::cisco_mcp::context;
use betterlite_guardrails::cisco_mcp::envelope;
use betterlite_guardrails::cisco_mcp::normalize;
use betterlite_guardrails::cisco_mcp::redact;
use serde::Deserialize;
use serde_json::{Map, Value, json};

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    RequestEnvelope {
        data: Map<String, Value>,
    },
    ResponseEnvelope {
        request_data: Map<String, Value>,
        response: Value,
    },
    HydrateContext {
        request_data: Map<String, Value>,
    },
    NormalizeDict {
        response: Value,
    },
    NormalizeModel {
        response: Value,
    },
    NormalizeFieldTuples {
        pairs: Value,
    },
    BlockingResponse {
        detail: Value,
    },
    RewriteText {
        response: Value,
        text: String,
    },
    RewriteModelText {
        response: Value,
        text: String,
    },
    RewriteFieldTuplesText {
        response: Value,
        text: String,
    },
    RewriteWrappedText {
        response: Value,
        text: String,
    },
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        Ok(json!({"result": outcome(input)}))
    }
}

fn outcome(input: &Input) -> Value {
    match input {
        Input::RequestEnvelope { data } => envelope::build_request_payload(
            &serde_json::from_value::<Fields>(Value::Object(data.clone())).unwrap_or_default(),
        )
        .map_or(Value::Null, |payload| {
            serde_json::from_str(&serde_json::to_string(&payload).unwrap_or_default())
                .unwrap_or(Value::Null)
        }),
        Input::ResponseEnvelope {
            request_data,
            response,
        } => response_envelope(request_data, response),
        Input::HydrateContext { request_data } => hydrate_context(request_data),
        Input::NormalizeDict { response } | Input::NormalizeModel { response } => {
            normalize_response(response)
        }
        Input::NormalizeFieldTuples { pairs } => normalize_response(pairs),
        Input::BlockingResponse { detail } => blocking_response(detail),
        Input::RewriteText { response, text }
        | Input::RewriteModelText { response, text }
        | Input::RewriteFieldTuplesText { response, text }
        | Input::RewriteWrappedText { response, text } => rewrite_text(response, text),
    }
}

fn response_envelope(request_data: &Map<String, Value>, response: &Value) -> Value {
    let fields: Fields =
        serde_json::from_value(Value::Object(request_data.clone())).unwrap_or_default();
    let parsed = Json::parse(&response.to_string());
    envelope::build_response_payload(&fields, &parsed).map_or(Value::Null, |payload| {
        serde_json::from_str(&serde_json::to_string(&payload).unwrap_or_default())
            .unwrap_or(Value::Null)
    })
}

fn hydrate_context(request_data: &Map<String, Value>) -> Value {
    let mut data: Fields =
        serde_json::from_value(Value::Object(request_data.clone())).unwrap_or_default();
    context::hydrate_tool_context(&mut data);
    serde_json::from_str(&serde_json::to_string(&data).unwrap_or_default()).unwrap_or(Value::Null)
}

fn normalize_response(response: &Value) -> Value {
    let parsed = Json::parse(&response.to_string());
    normalize::normalize_response(&parsed)
        .map_or(Value::Null, |normalized| normalized_to_value(&normalized))
}

fn blocking_response(detail: &Value) -> Value {
    let parsed = Json::parse(&detail.to_string());
    let response = blocking::build_blocking_response(
        &parsed,
        "cisco_ai_defense",
        Some("cisco-ai-defense"),
        None,
    );
    let mut output =
        serde_json::from_str::<Value>(&serde_json::to_string(&response).unwrap_or_default())
            .unwrap_or(Value::Null);
    if let Some(text) = output
        .pointer("/mcp_tool_call_response/0/text")
        .and_then(Value::as_str)
        .map(str::to_owned)
        && let Ok(payload) = serde_json::from_str(&text)
        && let Some(slot) = output.pointer_mut("/mcp_tool_call_response/0/text")
    {
        *slot = payload;
    }
    output
}

fn normalized_to_value(normalized: &normalize::NormalizedResponse) -> Value {
    let mut map = Map::new();
    map.insert("jsonrpc".to_owned(), json!("2.0"));
    let id = normalized.id_override.as_ref().map_or_else(
        || json!("litellm-mcp"),
        |value| {
            serde_json::from_str(&serde_json::to_string(value).unwrap_or_default())
                .unwrap_or(Value::Null)
        },
    );
    map.insert("id".to_owned(), id);
    if let Some(result) = &normalized.result {
        map.insert(
            "result".to_owned(),
            serde_json::from_str(&serde_json::to_string(result).unwrap_or_default())
                .unwrap_or(Value::Null),
        );
    }
    if let Some(error) = &normalized.error {
        map.insert(
            "error".to_owned(),
            serde_json::from_str(&serde_json::to_string(error).unwrap_or_default())
                .unwrap_or(Value::Null),
        );
    }
    Value::Object(map)
}

fn rewrite_text(response: &Value, text: &str) -> Value {
    let parsed = Json::parse(&response.to_string());
    let update = redact::set_tool_response_text(&parsed, text);
    json!({"replaced": update.replaced, "response": update.response})
}

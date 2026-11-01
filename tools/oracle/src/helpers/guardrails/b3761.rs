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
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        Ok(json!({"result": outcome(input)}))
    }
}

fn fields_to_value(source: &Fields) -> Value {
    let text = serde_json::to_string(source).unwrap_or_default();
    serde_json::from_str(&text).unwrap_or(Value::Null)
}

fn outcome(input: &Input) -> Value {
    match input {
        Input::RequestEnvelope { data } => {
            let fields: Fields =
                serde_json::from_value(Value::Object(data.clone())).unwrap_or_default();
            envelope::build_request_payload(&fields)
                .map_or(Value::Null, |payload| to_value(&payload))
        }
        Input::ResponseEnvelope {
            request_data,
            response,
        } => {
            let fields: Fields =
                serde_json::from_value(Value::Object(request_data.clone())).unwrap_or_default();
            let parsed = Json::parse(&response.to_string());
            envelope::build_response_payload(&fields, &parsed)
                .map_or(Value::Null, |payload| to_value(&payload))
        }
        Input::HydrateContext { request_data } => {
            let mut data: Fields =
                serde_json::from_value(Value::Object(request_data.clone())).unwrap_or_default();
            context::hydrate_tool_context(&mut data);
            fields_to_value(&data)
        }
        Input::NormalizeDict { response } | Input::NormalizeModel { response } => {
            let parsed = Json::parse(&response.to_string());
            normalize::normalize_response(&parsed)
                .map_or(Value::Null, |normalized| normalized_to_value(&normalized))
        }
        Input::NormalizeFieldTuples { pairs } => {
            let parsed = Json::parse(&pairs.to_string());
            normalize::normalize_response(&parsed)
                .map_or(Value::Null, |normalized| normalized_to_value(&normalized))
        }
        Input::BlockingResponse { detail } => {
            let parsed = Json::parse(&detail.to_string());
            let response = blocking::build_blocking_response(
                &parsed,
                "cisco_ai_defense",
                Some("cisco-ai-defense"),
                None,
            );
            to_value(&response)
        }
        Input::RewriteText { response, text } => rewrite_text(response, text),
    }
}

fn to_value<T>(value: &T) -> Value
where
    T: serde::Serialize,
{
    let text = serde_json::to_string(value).unwrap_or_default();
    serde_json::from_str(&text).unwrap_or(Value::Null)
}

fn normalized_to_value(normalized: &normalize::NormalizedResponse) -> Value {
    let mut map = Map::new();
    map.insert("jsonrpc".to_owned(), json!("2.0"));
    let id = normalized
        .id_override
        .as_ref()
        .map_or_else(|| json!("litellm-mcp"), to_value);
    map.insert("id".to_owned(), id);
    if let Some(result) = &normalized.result {
        map.insert("result".to_owned(), to_value(result));
    }
    if let Some(error) = &normalized.error {
        map.insert("error".to_owned(), to_value(error));
    }
    Value::Object(map)
}

fn rewrite_text(response: &Value, text: &str) -> Value {
    let parsed = Json::parse(&response.to_string());
    let update = redact::set_tool_response_text(&parsed, text);
    let mut rewritten = response.clone();
    if let Some(items) = &update.content
        && let Value::Array(_) = &rewritten
    {
        rewritten = to_value(items);
    }
    if update.structured_content.is_some()
        && let Value::Object(fields) = &mut rewritten
    {
        let key = if fields.contains_key("structured_content") {
            "structured_content"
        } else {
            "structuredContent"
        };
        fields.insert(key.to_owned(), json!({"result": text}));
    }
    json!({"replaced": update.replaced, "response": rewritten})
}

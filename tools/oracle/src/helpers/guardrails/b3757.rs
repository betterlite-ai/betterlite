use betterlite_guardrails::cisco_conversation::Fields;
use betterlite_guardrails::cisco_conversation::json::Json;
use betterlite_guardrails::cisco_mcp::redact;
use betterlite_guardrails::cisco_redaction::chat_input::redact_chat_input;
use betterlite_guardrails::cisco_redaction::chat_output::list_field::{
    ListField, redact_list_field,
};
use betterlite_guardrails::cisco_redaction::chat_output::redact_model_response;
use betterlite_guardrails::cisco_redaction::mcp::{
    apply_redaction_mcp_input, apply_redaction_mcp_output,
};
use betterlite_guardrails::cisco_verdict;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Call {
    ApplyRedactionMcpInput {
        #[serde(rename = "request_data")]
        data: Value,
        #[serde(rename = "sanitized_text")]
        text: Option<String>,
        #[serde(rename = "sanitized_mcp_arguments")]
        args: Option<Value>,
    },
    ApplyRedactionChatInput {
        #[serde(rename = "request_data")]
        data: Value,
        #[serde(rename = "sanitized_text")]
        text: Option<String>,
        #[serde(rename = "sanitized_messages")]
        msgs: Option<Vec<Value>>,
    },
    #[serde(rename = "apply_redaction_chat_output_model_response")]
    ApplyRedactionModelResponse {
        choices: Value,
        #[serde(rename = "sanitized_text")]
        text: Option<String>,
        #[serde(rename = "sanitized_messages")]
        msgs: Option<Vec<Value>>,
    },
    #[serde(rename = "apply_redaction_chat_output_text_completion")]
    ApplyRedactionTextCompletion {
        choices: Value,
        #[serde(rename = "sanitized_text")]
        text: Option<String>,
        #[serde(rename = "sanitized_messages")]
        msgs: Option<Vec<Value>>,
    },
    #[serde(rename = "apply_redaction_responses_api_output")]
    ApplyRedactionResponsesOutput {
        response: Value,
        #[serde(rename = "sanitized_text")]
        text: Option<String>,
        #[serde(rename = "sanitized_messages")]
        msgs: Option<Vec<Value>>,
    },
    ApplyRedactionMcpOutput {
        response: Option<Value>,
        #[serde(rename = "sanitized_text")]
        text: Option<String>,
    },
    ExtractMaskedEntityCountRules {
        rules: Value,
    },
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Call;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        Ok(json!({"result": outcome(input)}))
    }
}

fn outcome(call: &Call) -> Value {
    match call {
        Call::ApplyRedactionMcpInput { data, text, args } => {
            mcp_input(data, text.as_deref(), args.as_ref())
        }
        Call::ApplyRedactionChatInput { data, text, msgs } => {
            chat_input(data, text.as_deref(), msgs.as_deref())
        }
        Call::ApplyRedactionModelResponse {
            choices,
            text,
            msgs,
        } => chat_output_choices(
            choices,
            text.as_deref(),
            msgs.as_deref(),
            ChatOutputKind::ModelResponse,
        ),
        Call::ApplyRedactionTextCompletion {
            choices,
            text,
            msgs,
        } => chat_output_choices(
            choices,
            text.as_deref(),
            msgs.as_deref(),
            ChatOutputKind::TextCompletion,
        ),
        Call::ApplyRedactionResponsesOutput {
            response,
            text,
            msgs,
        } => responses_api_output(response, text.as_deref(), msgs.as_deref()),
        Call::ApplyRedactionMcpOutput { response, text } => {
            mcp_output(response.as_ref(), text.as_deref())
        }
        Call::ExtractMaskedEntityCountRules { rules } => masked_entity_count_rules(rules),
    }
}

fn responses_api_output(response: &Value, text: Option<&str>, msgs: Option<&[Value]>) -> Value {
    let mut fields = fields_of(response);
    let applied = redact_list_field(
        Some(&mut fields),
        text,
        convert_messages(msgs).as_deref(),
        ListField::ResponsesOutput,
    );
    json!({"applied": applied, "response": to_value(&fields)})
}

fn masked_entity_count_rules(rules: &Value) -> Value {
    cisco_verdict::extract_masked_entity_count(&Json::parse(&rules.to_string()))
        .map_or(Value::Null, |counts| json!(counts))
}

fn convert_messages(sanitized_messages: Option<&[Value]>) -> Option<Vec<Fields>> {
    let messages = sanitized_messages?;
    let mut converted = Vec::with_capacity(messages.len());
    for message in messages {
        converted.push(fields_of(message));
    }
    Some(converted)
}

fn mcp_input(data: &Value, text: Option<&str>, arguments: Option<&Value>) -> Value {
    let converted_arguments = arguments.map(fields_of);
    let mut fields = fields_of(data);
    let applied = apply_redaction_mcp_input(&mut fields, text, converted_arguments.as_ref());
    json!({"applied": applied, "request_data": to_value(&fields)})
}

fn chat_input(data: &Value, text: Option<&str>, messages: Option<&[Value]>) -> Value {
    let converted_messages = convert_messages(messages);
    let mut fields = fields_of(data);
    let applied = redact_chat_input(&mut fields, text, converted_messages.as_deref());
    json!({"applied": applied, "request_data": to_value(&fields)})
}

#[derive(Clone, Copy)]
enum ChatOutputKind {
    ModelResponse,
    TextCompletion,
}

fn chat_output_choices(
    choices: &Value,
    text: Option<&str>,
    messages: Option<&[Value]>,
    kind: ChatOutputKind,
) -> Value {
    let converted_messages = convert_messages(messages);
    let mut fields = fields_of(&json!({"choices": choices}));
    let (applied, result_key, field_key) = match kind {
        ChatOutputKind::ModelResponse => (
            redact_model_response(Some(&mut fields), text, converted_messages.as_deref()),
            "messages",
            "message",
        ),
        ChatOutputKind::TextCompletion => (
            redact_list_field(
                Some(&mut fields),
                text,
                converted_messages.as_deref(),
                ListField::TextCompletion,
            ),
            "texts",
            "text",
        ),
    };
    json!({"applied": applied, (result_key): choice_sub_field(&fields, field_key)})
}

fn mcp_output(response: Option<&Value>, sanitized_text: Option<&str>) -> Value {
    let Some(present_response) = response else {
        return json!({"applied": false, "response": Value::Null});
    };
    let parsed = Json::parse(&present_response.to_string());
    let Some(update) = apply_redaction_mcp_output(Some(&parsed), sanitized_text) else {
        return json!({"applied": false, "response": present_response});
    };
    json!({"applied": update.replaced, "response": rewrite_mcp_output(present_response, &update)})
}

fn rewrite_mcp_output(response: &Value, update: &redact::TextUpdate) -> Value {
    let mut rewritten = response.clone();
    if let Some(items) = &update.content
        && let Value::Array(_) = rewritten
    {
        rewritten = to_value(items);
    }
    if let Some(mirror) = &update.structured_content
        && let Value::Object(fields) = &mut rewritten
    {
        let key = if fields.contains_key("structured_content") {
            "structured_content"
        } else {
            "structuredContent"
        };
        fields.insert(key.to_owned(), to_value(mirror));
    }
    rewritten
}

fn choice_sub_field(response: &Fields, key: &str) -> Value {
    let Json::List(raw_choices) = Json::field(response, "choices") else {
        return Value::Array(Vec::new());
    };
    Value::Array(
        raw_choices
            .iter()
            .map(|raw_choice| {
                let Json::Object(choice) = Json::parse(raw_choice.get()) else {
                    return Value::Null;
                };
                json_to_value(&Json::field(&choice, key))
            })
            .collect(),
    )
}

fn json_to_value(value: &Json) -> Value {
    match value {
        Json::Null => Value::Null,
        Json::Bool(flag) => Value::Bool(*flag),
        Json::Number(text) => serde_json::from_str(text).unwrap_or(Value::Null),
        Json::Text(text) => Value::String(text.clone()),
        Json::List(items) => Value::Array(
            items
                .iter()
                .map(|item| serde_json::from_str(item.get()).unwrap_or(Value::Null))
                .collect(),
        ),
        Json::Object(fields) => to_value(fields),
    }
}

fn fields_of(value: &Value) -> Fields {
    match Json::parse(&value.to_string()) {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Fields::default()
        }
    }
}

fn to_value<T>(value: &T) -> Value
where
    T: serde::Serialize,
{
    let Ok(text) = serde_json::to_string(value) else {
        return Value::Null;
    };
    serde_json::from_str(&text).unwrap_or(Value::Null)
}

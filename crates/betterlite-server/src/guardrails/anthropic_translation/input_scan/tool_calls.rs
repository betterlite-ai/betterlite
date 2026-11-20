use betterlite_logging::python_json::dumps;
use betterlite_models::decisions::Json;
use serde_json::{Map, Value};

use crate::guardrails::anthropic_translation::scan_targets::is_client_tool_use;

const ARGUMENTS: &str = "arguments";
const CALLER: &str = "caller";
const FUNCTION: &str = "function";
const FUNCTION_TYPE: &str = "function";
const ID: &str = "id";
const INDEX: &str = "index";
const INPUT: &str = "input";
const NAME: &str = "name";
const TYPE: &str = "type";

// LiteLLM's convert_tool_use_to_openai_format: the chat tool call a guardrail sees for a client tool_use block
pub(crate) fn client_tool_use_to_openai(block: &Map<String, Value>, index: usize) -> Option<Value> {
    if !is_client_tool_use(block) {
        return None;
    }
    output_tool_use_to_openai(block, index)
}

// The output side accepts any tool_use block, so a missing id, name or input is the caller's error to raise
pub(crate) fn output_tool_use_to_openai(block: &Map<String, Value>, index: usize) -> Option<Value> {
    let id = block.get(ID)?.as_str()?;
    let name = block.get(NAME)?.as_str()?;
    Some(tool_call_chunk(
        id,
        name,
        block.get(INPUT)?,
        index,
        block.get(CALLER),
    ))
}

fn tool_call_chunk(
    id: &str,
    name: &str,
    input: &Value,
    index: usize,
    caller: Option<&Value>,
) -> Value {
    let mut tool_call = Map::new();
    tool_call.insert(ID.to_owned(), Value::String(id.to_owned()));
    tool_call.insert(TYPE.to_owned(), Value::String(FUNCTION_TYPE.to_owned()));
    tool_call.insert(FUNCTION.to_owned(), function_chunk(name, input));
    tool_call.insert(INDEX.to_owned(), Value::from(index));
    if let Some(caller_value) = caller {
        tool_call.insert(CALLER.to_owned(), caller_value.clone());
    }
    Value::Object(tool_call)
}

fn function_chunk(name: &str, input: &Value) -> Value {
    let mut function = Map::new();
    function.insert(NAME.to_owned(), Value::String(name.to_owned()));
    function.insert(
        ARGUMENTS.to_owned(),
        Value::String(dumps(&decision_json(input))),
    );
    Value::Object(function)
}

// Python's json.dumps writes the arguments string a guardrail reads, escaping and formatting as LiteLLM does
fn decision_json(value: &Value) -> Json {
    match value {
        Value::Null => Json::Null,
        Value::Bool(flag) => Json::Bool(*flag),
        Value::Number(number) => Json::Number(number.clone()),
        Value::String(text) => Json::Text(text.clone()),
        Value::Array(items) => Json::List(items.iter().map(decision_json).collect()),
        Value::Object(fields) => Json::Object(
            fields
                .iter()
                .map(|(field, item)| (field.clone(), decision_json(item)))
                .collect(),
        ),
    }
}

use betterlite_core::api::responses_tool_repair::definition::{
    append_tool_call, create_tool_call_chunk, normalize_definition, reconstruct_from_tools,
};
use betterlite_core::api::responses_tool_repair::error::RepairError;
use betterlite_core::api::responses_tool_repair::value::Value;
use betterlite_core::api::responses_tool_repair::{
    call_exists, recover_call_id, repair_tool_results, tool_calls_of,
};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Input {
    Repair {
        messages: Vec<Value>,
        tools: Vec<Value>,
        cache: IndexMap<String, Value>,
    },
    Recover {
        assistant: Value,
    },
    Calls {
        assistant: Value,
        tool_call_id: String,
    },
    Field {
        target: Value,
        key: String,
    },
    Chunk {
        definition: Value,
        tool_call_id: String,
        index: usize,
    },
    Normalize {
        definition: Value,
        tool_call_id: String,
    },
    Reconstruct {
        tool_call_id: String,
        tools: Vec<Value>,
    },
    Append {
        assistant: Value,
        chunk: Value,
    },
}

fn operate(input: &Input) -> Result<serde_json::Value, RepairError> {
    match input {
        Input::Repair {
            messages,
            tools,
            cache,
        } => Ok(json!({
            "repaired": repair_tool_results(messages, tools, cache)?,
            "input": messages,
        })),
        Input::Recover { assistant } => Ok(json!({"id": recover_call_id(assistant)?})),
        Input::Calls {
            assistant,
            tool_call_id,
        } => {
            let calls = tool_calls_of(assistant);
            Ok(json!({"exists": call_exists(&calls, tool_call_id), "calls": calls}))
        }
        Input::Field { target, key } => Ok(json!({"value": target.field(key)?})),
        Input::Chunk {
            definition,
            tool_call_id,
            index,
        } => Ok(json!({"chunk": create_tool_call_chunk(definition, tool_call_id, *index)?})),
        Input::Normalize {
            definition,
            tool_call_id,
        } => Ok(json!({"definition": normalize_definition(definition, tool_call_id)?})),
        Input::Reconstruct {
            tool_call_id,
            tools,
        } => Ok(json!({"definition": reconstruct_from_tools(tool_call_id, tools)?})),
        Input::Append { assistant, chunk } => {
            let mut appended = assistant.clone();
            append_tool_call(&mut appended, chunk.clone());
            Ok(json!({"assistant": appended}))
        }
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        // LiteLLM's helper reports a failing getter or a missing get method as a name, not a Python exception
        let outcome = operate(input).unwrap_or_else(|error| match error {
            RepairError::NoGetMethod => json!({"error": "no_get_method"}),
            RepairError::GetterFailed => json!({"error": "getter_failed"}),
            RepairError::WrongType { field } => json!({"error": "wrong_type", "field": field}),
        });
        Ok(outcome)
    }
}

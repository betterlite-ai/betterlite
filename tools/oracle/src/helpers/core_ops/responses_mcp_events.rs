use std::collections::BTreeMap;

use betterlite_core::api::responses_mcp_events::call::{McpCallRequest, create_mcp_call_events};
use betterlite_core::api::responses_mcp_events::error::McpEventError;
use betterlite_core::api::responses_mcp_events::event::{EventField, StreamEvent};
use betterlite_core::api::responses_mcp_events::function_call::function_call_id;
use betterlite_core::api::responses_mcp_events::list_tools::{
    DiagnosticLevel, DiscoveredTool, ServerEntry, ToolAttribute, create_mcp_list_tools_events,
};
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Input {
    OutputItemsAndFunctionCallId {
        output: Vec<Value>,
    },
    SetEventField {
        field: String,
        value: Value,
        #[serde(default)]
        event: Option<BTreeMap<String, Value>>,
    },
    ListTools {
        servers: Vec<ServerEntry>,
        base_item_id: String,
        tools: Vec<ToolInput>,
    },
    McpCall {
        #[serde(flatten)]
        request: McpCallRequest,
        #[serde(default)]
        ids: Vec<String>,
    },
}

#[derive(Debug, Deserialize)]
pub(crate) struct ToolInput {
    #[serde(default)]
    raises: BTreeMap<String, String>,
    #[serde(flatten)]
    attributes: BTreeMap<String, Value>,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::OutputItemsAndFunctionCallId { output } => {
                let response: ResponsesApiResponse = serde_json::from_value(
                    json!({"id": "resp_1", "created_at": 1, "output": output}),
                )
                .map_err(|error| helper_error(&error.to_string()))?;
                let identifiers: Vec<Option<&str>> =
                    response.output.iter().map(function_call_id).collect();
                Ok(json!(identifiers))
            }
            Input::SetEventField {
                field,
                value,
                event,
            } => assign_event_field(field, value, event.as_ref()),
            Input::ListTools {
                servers,
                base_item_id,
                tools,
            } => Ok(list_tools_projection(servers, base_item_id, tools)),
            Input::McpCall { request, ids } => {
                let events = call_events(request, ids.clone())?;
                Ok(json!({"events": events, "diagnostics": []}))
            }
        }
    }
}

fn assign_event_field(
    field: &str,
    value: &Value,
    event: Option<&BTreeMap<String, Value>>,
) -> crate::helpers::HelperResult {
    let mut target = if let Some(fields) = event {
        StreamEvent {
            fields: fields
                .iter()
                .map(|(name, content)| (name.clone(), EventField::Json(content.clone())))
                .collect(),
        }
    } else {
        let sample = McpCallRequest {
            tool_name: "w".to_owned(),
            arguments: "{}".to_owned(),
            result: None,
            base_item_id: Some("mcp_x".to_owned()),
            sequence_start: 1,
            output_index: 1,
        };
        call_events(&sample, Vec::new())?.remove(0)
    };
    target
        .fields
        .insert(field.to_owned(), EventField::Json(value.clone()));
    Ok(json!(target))
}

fn list_tools_projection(
    servers: &[ServerEntry],
    base_item_id: &str,
    tools: &[ToolInput],
) -> Value {
    let discovered: Vec<DiscoveredTool> = tools.iter().map(discovered_tool).collect();
    let outcome = create_mcp_list_tools_events(servers, base_item_id, &discovered);
    let diagnostics: Vec<Value> = outcome
        .diagnostics
        .iter()
        .map(|diagnostic| {
            let level = match diagnostic.level {
                DiagnosticLevel::Debug => "debug",
                DiagnosticLevel::Error => "error",
                DiagnosticLevel::Traceback => "traceback",
            };
            json!({"level": level, "message": diagnostic.message})
        })
        .collect();
    json!({"events": outcome.events, "diagnostics": diagnostics})
}

fn call_events(
    request: &McpCallRequest,
    ids: Vec<String>,
) -> Result<Vec<StreamEvent>, Box<crate::helpers::HelperError>> {
    let mut remaining = ids.into_iter();
    create_mcp_call_events(request, || remaining.next().unwrap_or_default())
        .map_err(|error: McpEventError| helper_error(&error.to_string()))
}

fn discovered_tool(tool: &ToolInput) -> DiscoveredTool {
    DiscoveredTool {
        name: attribute(tool, "name", |value| value.as_str().map(str::to_owned)),
        description: attribute(tool, "description", |value| {
            value.as_str().map(str::to_owned)
        }),
        input_schema: attribute(tool, "input_schema", Value::clone),
    }
}

fn attribute<Content, Read>(tool: &ToolInput, field: &str, read: Read) -> ToolAttribute<Content>
where
    Read: Fn(&Value) -> Content,
{
    if let Some(message) = tool.raises.get(field) {
        return ToolAttribute::Raises(message.clone());
    }
    tool.attributes
        .get(field)
        .map_or(ToolAttribute::Absent, |value| {
            ToolAttribute::Value(read(value))
        })
}

fn helper_error(message: &str) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}

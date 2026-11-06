use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::Message;
use betterlite_prompts::response_tool_calls::normalized::get_tool_calls_from_response;
use betterlite_prompts::response_tool_calls::projections::ToolCallResponse;
use betterlite_prompts::response_tool_calls::structured::{
    MappedHandler, RouteCallTypes, StructuredMessageHandler, resolve_structured_messages,
};
use betterlite_prompts::response_tool_calls::tools::has_tool_with_name;
use serde::Deserialize;
use serde_json::json;

use crate::helpers::{HelperError, HelperResult};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    GetToolCalls {
        response: ToolCallResponse,
        #[serde(default)]
        include_all_choices: bool,
    },
    HasTool {
        tools: Vec<Json>,
        tool_name: String,
    },
    ResolveStructuredMessages {
        calls: Vec<StructuredCall>,
    },
}

#[derive(Debug, Deserialize)]
pub(crate) struct StructuredCall {
    #[serde(default)]
    messages: Option<Vec<Message>>,
    request_kwargs: Object,
    #[serde(default)]
    routes: Vec<RouteInput>,
    #[serde(default)]
    handlers: Vec<HandlerInput>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RouteInput {
    route: String,
    call_types: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct HandlerInput {
    call_types: Vec<String>,
    #[serde(default)]
    messages: Option<Vec<Message>>,
}

struct ScriptedHandler {
    messages: Option<Vec<Message>>,
}

impl StructuredMessageHandler for ScriptedHandler {
    fn get_structured_messages(&self, _request_kwargs: &Object) -> Option<Vec<Message>> {
        self.messages.clone()
    }
}

struct ScriptedRoutes {
    routes: Vec<(String, Vec<String>)>,
}

impl RouteCallTypes<String> for ScriptedRoutes {
    fn call_types_for_route(&self, route: &str) -> Option<Vec<String>> {
        self.routes
            .iter()
            .find(|(name, _)| name == route)
            .map(|(_, call_types)| call_types.clone())
    }
}

fn resolve_call(call: &StructuredCall) -> Option<Vec<Message>> {
    let handlers: Vec<ScriptedHandler> = call
        .handlers
        .iter()
        .map(|input| ScriptedHandler {
            messages: input.messages.clone(),
        })
        .collect();
    let keys: Vec<Vec<String>> = call
        .handlers
        .iter()
        .map(|input| input.call_types.clone())
        .collect();
    let mappings: Vec<MappedHandler<'_, String>> = keys
        .iter()
        .zip(&handlers)
        .map(|(call_types, handler)| MappedHandler {
            call_types,
            handler,
        })
        .collect();
    let routes = ScriptedRoutes {
        routes: call
            .routes
            .iter()
            .map(|route| (route.route.clone(), route.call_types.clone()))
            .collect(),
    };
    resolve_structured_messages(
        call.messages.as_deref(),
        &call.request_kwargs,
        &mappings,
        &routes,
    )
}

fn resolve_calls(calls: &[StructuredCall]) -> Vec<Option<Vec<Message>>> {
    let mut results = Vec::new();
    for call in calls {
        results.push(resolve_call(call));
    }
    results
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::GetToolCalls {
                response,
                include_all_choices,
            } => serde_json::to_value(get_tool_calls_from_response(response, *include_all_choices))
                .map_err(|error| {
                    Box::new(HelperError {
                        class: "SerializationError".to_owned(),
                        status: None,
                        error_type: None,
                        param: None,
                        code: None,
                        message: error.to_string(),
                    })
                }),
            Input::HasTool { tools, tool_name } => Ok(json!(has_tool_with_name(tools, tool_name))),
            Input::ResolveStructuredMessages { calls } => {
                serde_json::to_value(resolve_calls(calls)).map_err(|error| {
                    Box::new(HelperError {
                        class: "SerializationError".to_owned(),
                        status: None,
                        error_type: None,
                        param: None,
                        code: None,
                        message: error.to_string(),
                    })
                })
            }
        }
    }
}

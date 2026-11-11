use std::collections::BTreeSet;

use betterlite_openapi::error_responses::{error_response_schema, error_responses};
use betterlite_openapi::llm_request_bodies::{LlmRequestSchemas, add_llm_request_bodies};
use betterlite_openapi::operation_ids::{ensure_unique_operation_ids, stable_operation_id};
use betterlite_openapi::schema_transforms::tree::JsonObject;
use betterlite_openapi::websocket_stubs::{WebsocketRoute, inject_websocket_stubs};

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    ErrorResponseTable,
    ErrorResponseSchema,
    StableOperationId {
        name: String,
        path_format: String,
        methods: Vec<String>,
    },
    EnsureUniqueOperationIds {
        document: JsonObject,
        reserved: Option<Vec<String>>,
    },
    InjectWebsocketStubs {
        document: JsonObject,
        routes: Vec<WebsocketRoute>,
    },
    AddLlmRequestBodies {
        document: JsonObject,
        schemas: LlmRequestSchemas,
    },
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> crate::helpers::HelperResult {
        match input {
            Input::ErrorResponseTable => to_json(error_responses()),
            Input::ErrorResponseSchema => to_json(error_response_schema()),
            Input::StableOperationId {
                name,
                path_format,
                methods,
            } => {
                let method_names = methods.iter().map(String::as_str).collect::<Vec<&str>>();
                Ok(serde_json::Value::String(stable_operation_id(
                    name,
                    path_format,
                    &method_names,
                )))
            }
            Input::EnsureUniqueOperationIds { document, reserved } => {
                let mut updated = document.clone();
                let mut reserved_ids = reserved
                    .as_ref()
                    .map(|ids| ids.iter().cloned().collect::<BTreeSet<String>>());
                ensure_unique_operation_ids(&mut updated, reserved_ids.as_mut());
                let reserved_after: Option<Vec<String>> =
                    reserved_ids.map(|ids| ids.into_iter().collect());
                Ok(serde_json::json!({
                    "document": updated,
                    "reserved": reserved_after,
                }))
            }
            Input::InjectWebsocketStubs { document, routes } => {
                let mut updated = document.clone();
                inject_websocket_stubs(&mut updated, routes);
                to_json(updated)
            }
            Input::AddLlmRequestBodies { document, schemas } => {
                let mut updated = document.clone();
                add_llm_request_bodies(&mut updated, schemas);
                to_json(updated)
            }
        }
    }
}

fn to_json<T>(value: T) -> crate::helpers::HelperResult
where
    T: serde::Serialize,
{
    serde_json::to_value(value).map_err(|error| {
        Box::new(crate::helpers::HelperError {
            class: "SerializationError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })
}

use std::collections::{BTreeMap, BTreeSet};

use betterlite_guardrails::straiker_webhook::context::{
    CatalogProviderNames, call_surface, destination_host, model_provider,
};
use betterlite_guardrails::straiker_webhook::failure::status_failure;
use betterlite_guardrails::straiker_webhook::jsonable::{
    jsonable_dict, opaque_dict_list, record_dict,
};
use betterlite_guardrails::straiker_webhook::metadata::{extract_identity, webhook_metadata};
use betterlite_guardrails::straiker_webhook::modes::{ModeError, configured_modes};
use betterlite_guardrails::straiker_webhook::responses::{response_finish_reason, response_usage};
use betterlite_guardrails::straiker_webhook::streamed::request_is_streamed;
use betterlite_guardrails::straiker_webhook::structured_messages::request_structured_messages;
use betterlite_guardrails::straiker_webhook::types::StraikerWebhookUsage;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::response_tool_calls::structured::RouteCallTypes;
use serde::Deserialize;
use serde_json::json;

use crate::helpers::{HelperError, HelperResult};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Identity {
        request_data: Object,
    },
    WebhookMetadata {
        request_data: Object,
        default_metadata: BTreeMap<String, String>,
    },
    Provider {
        request_data: Object,
        model: Option<String>,
    },
    Destination {
        request_data: Object,
    },
    CallSurface {
        request_data: Object,
        logging_obj: Option<LoggingObject>,
    },
    FinishReason {
        response: Json,
    },
    FinishReasonObject {
        response: Json,
    },
    Usage {
        response: Json,
    },
    ConfiguredModes {
        event_hook: Option<Json>,
    },
    StructuredMessages {
        request_data: Object,
    },
    Jsonable {
        item: JsonableItem,
    },
    OpaqueDictList {
        items: Vec<JsonableItem>,
    },
    Streamed {
        request_data: Object,
    },
    StatusFailure {
        status: u16,
        text: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LoggingObject {
    call_type: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum JsonableItem {
    Plain(Json),
    Model(StraikerWebhookUsage),
}

struct NoRoutes;

impl RouteCallTypes<String> for NoRoutes {
    fn call_types_for_route(&self, _route: &str) -> Option<Vec<String>> {
        None
    }
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Identity { request_data } => Ok(json!(extract_identity(request_data))),
            Input::WebhookMetadata {
                request_data,
                default_metadata,
            } => Ok(json!(webhook_metadata(request_data, default_metadata))),
            Input::Provider {
                request_data,
                model,
            } => Ok(provider(request_data, model.as_deref())),
            Input::Destination { request_data } => Ok(json!(destination_host(request_data))),
            Input::CallSurface {
                request_data,
                logging_obj,
            } => {
                let call_type = logging_obj
                    .as_ref()
                    .and_then(|configured| configured.call_type.as_deref());
                Ok(json!(call_surface(call_type, request_data)))
            }
            Input::FinishReason { response } | Input::FinishReasonObject { response } => {
                Ok(json!(response_finish_reason(response)))
            }
            Input::Usage { response } => Ok(json!(response_usage(response))),
            Input::ConfiguredModes { event_hook } => {
                let modes =
                    configured_modes(event_hook.as_ref()).map_err(|error| mode_error(&error))?;
                Ok(json!(modes))
            }
            Input::StructuredMessages { request_data } => Ok(json!(request_structured_messages(
                request_data,
                &NoRoutes,
                &[]
            ))),
            Input::Jsonable { item } => Ok(json!(jsonable(item))),
            Input::OpaqueDictList { items } => Ok(json!(opaque_list(items))),
            Input::Streamed { request_data } => Ok(json!(request_is_streamed(request_data))),
            Input::StatusFailure { status, text } => Ok(json!(status_failure(*status, text))),
        }
    }
}

fn provider(request_data: &Object, model: Option<&str>) -> serde_json::Value {
    let catalog = ModelCatalog::default();
    let known_model_names = BTreeSet::new();
    let custom_providers = BTreeSet::new();
    let environment = BTreeMap::new();
    let registry = JSONProviderRegistry::load();
    let names = CatalogProviderNames {
        catalog: &catalog,
        known_model_names: &known_model_names,
        custom_providers: &custom_providers,
        environment: &environment,
        registry: &registry,
    };
    json!(model_provider(request_data, model, &names))
}

fn mode_error(error: &ModeError) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValidationError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn jsonable(item: &JsonableItem) -> Option<Object> {
    match item {
        JsonableItem::Plain(value) => jsonable_dict(value),
        JsonableItem::Model(model) => record_dict(model),
    }
}

fn opaque_list(items: &[JsonableItem]) -> Option<Vec<Object>> {
    let mut list = Vec::with_capacity(items.len());
    for item in items {
        list.push(match item {
            JsonableItem::Plain(value) => value.clone(),
            JsonableItem::Model(model) => {
                record_dict(model).map_or_else(|| Json::Null, Json::Object)
            }
        });
    }
    opaque_dict_list(&Json::List(list))
}

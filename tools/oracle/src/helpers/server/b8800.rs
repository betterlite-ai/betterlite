use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};
use betterlite_server::pre_call::failure::PreCallFailure;
use betterlite_server::pre_call::rewrites::{
    RequestModel, file_search_store_ids, request_model, stream_usage_updates,
};
use betterlite_server::pre_call::route_type::RouteType;

mod tests;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    function: Function,
    data: Map<String, Value>,
    #[serde(default)]
    always_include: Option<bool>,
    #[serde(default = "default_route_type")]
    route_type: String,
    #[serde(default)]
    supports: bool,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    StreamUsageUpdates,
    FileSearchStoreIds,
    RequestModel,
}

fn default_route_type() -> String {
    "acompletion".to_owned()
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input.function {
            Function::StreamUsageUpdates => stream_usage(input),
            Function::FileSearchStoreIds => file_search_store_ids(&input.data)
                .map_err(failure)
                .map(|ids| json!(ids)),
            Function::RequestModel => Ok(model_of(&input.data)),
        }
    }
}

fn stream_usage(input: &Input) -> HelperResult {
    let route = input
        .route_type
        .parse::<RouteType>()
        .map_err(|error| value_error(format!("unknown route type {error}")))?;
    let updates = stream_usage_updates(&input.data, input.always_include, route, || input.supports);
    Ok(Value::Object(updates))
}

fn model_of(data: &Map<String, Value>) -> Value {
    match request_model(data) {
        RequestModel::One(model) => Value::String(model),
        RequestModel::Many(models) => json!(models),
        RequestModel::Absent => Value::Null,
    }
}

fn value_error(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn failure(error: PreCallFailure) -> Box<HelperError> {
    let (class, status, message) = match error {
        PreCallFailure::Proxy(proxy) => ("ProxyException", proxy.code, proxy.message),
        PreCallFailure::RateLimited(limited) => {
            ("ProxyRateLimitError", limited.code, limited.message)
        }
        PreCallFailure::Http { status, detail } => ("HTTPException", status, detail.to_string()),
    };
    Box::new(HelperError {
        class: class.to_owned(),
        status: Some(status),
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

use std::collections::BTreeMap;

use betterlite_openai::response_input::ResponsesInput;
use betterlite_openai::response_resources::request::{ListInputItemsQuery, ResourceOperation};
use betterlite_prompts::arguments::json::Object;
use betterlite_provider::azure_resource_dispatch::responses::{self, Call, Outcome};
use betterlite_provider::azure_resource_dispatch::{
    AzureResourceError, AzureResourceOperation, AzureResourceResult, dispatch,
};
use serde_json::{Map, Value, json};

use super::super::support::{OTHER, failure, runtime};
use super::{Answer, InputItemsStep, Step};

const PROJECTED: [&str; 3] = ["model", "input", "tools"];

// B544's transform, URL and resource calls; every reply decoder stays with the leaf
pub(super) fn answer(step: &Step) -> Answer {
    let input = ResponsesInput::Text(text_of(step));
    let options: Object = Object::new();
    let query = query_of(step);
    let (kind, call) = match step {
        Step::ResponsesUrl(value) => (
            "responses_url",
            Call::CompleteUrl {
                api_base: Some(&value.api_base),
                api_version: value.api_version.as_deref(),
            },
        ),
        Step::ResponsesTransform(value) => (
            "responses_transform",
            Call::Transform {
                model: &value.model,
                input: &input,
                options: &options,
                model_info: None,
            },
        ),
        Step::ResponsesResource(value) => (
            "responses_resource",
            Call::Resource {
                api_base: &value.api_base,
                response_id: &value.response_id,
                operation: operation_of(&value.endpoint),
            },
        ),
        Step::ResponsesInputItems(value) => (
            "responses_input_items",
            Call::ListInputItems {
                api_base: &value.api_base,
                response_id: &value.response_id,
                query: &query,
            },
        ),
        _ => return Err(failure(OTHER)),
    };
    let environment: BTreeMap<String, String> = BTreeMap::new();
    let runtime = runtime()?;
    let result = runtime.block_on(dispatch(AzureResourceOperation::Responses(Box::new(
        responses::Dispatch {
            environment: &environment,
            call,
        },
    ))));
    let value = match result {
        Ok(AzureResourceResult::Responses(Outcome::Url(url))) => json!({"url": url}),
        Ok(AzureResourceResult::Responses(Outcome::Transformed(object))) => {
            json!({"projected": project(&object)})
        }
        Ok(AzureResourceResult::Responses(Outcome::Resource(request))) => {
            json!({"url": request.url, "data": request.payload})
        }
        Ok(_) => return Err(failure(OTHER)),
        Err(error) => return Err(failure(error)),
    };
    Ok((kind, value))
}

fn text_of(step: &Step) -> String {
    match step {
        Step::ResponsesTransform(value) => value.input.clone(),
        _ => String::new(),
    }
}

fn query_of(step: &Step) -> ListInputItemsQuery {
    let Step::ResponsesInputItems(value) = step else {
        return ListInputItemsQuery::default();
    };
    let InputItemsStep {
        after,
        before,
        include,
        limit,
        order,
        ..
    } = value;
    ListInputItemsQuery {
        after: after.clone(),
        before: before.clone(),
        include: include.clone(),
        limit: *limit,
        order: *order,
    }
}

fn operation_of(endpoint: &str) -> ResourceOperation {
    match endpoint {
        "get" => ResourceOperation::Get,
        "delete" => ResourceOperation::Delete,
        _ => ResourceOperation::Cancel,
    }
}

fn project(object: &Object) -> Map<String, Value> {
    let mut projected = Map::new();
    for name in PROJECTED {
        if let Some(value) = object.get(name) {
            projected.insert(
                String::from(name),
                serde_json::to_value(value).unwrap_or(Value::Null),
            );
        }
    }
    projected
}

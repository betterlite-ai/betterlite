use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::pin::Pin;

use betterlite_azure::responses::error::ResponsesError;
use betterlite_azure::responses::request::transform_request;
use betterlite_azure::responses::resources::{list_input_items, resource_request};
use betterlite_azure::responses::url::{complete_url, websocket_url};
use betterlite_openai::response_input::ResponsesInput;
use betterlite_openai::response_resources::request::{
    ListInputItemsQuery, ResourceOperation, ResourceRequest,
};
use betterlite_prompts::arguments::json::Object;

// The environment only feeds the responses api-version default, so a flat map is enough
pub struct Dispatch<'call> {
    pub environment: &'call BTreeMap<String, String>,
    pub call: Call<'call>,
}

pub enum Call<'call> {
    Transform {
        model: &'call str,
        input: &'call ResponsesInput,
        options: &'call Object,
        model_info: Option<&'call Object>,
    },
    CompleteUrl {
        api_base: Option<&'call str>,
        api_version: Option<&'call str>,
    },
    WebsocketUrl {
        api_base: Option<&'call str>,
    },
    Resource {
        api_base: &'call str,
        response_id: &'call str,
        operation: ResourceOperation,
    },
    ListInputItems {
        api_base: &'call str,
        response_id: &'call str,
        query: &'call ListInputItemsQuery,
    },
}

pub enum Outcome {
    Transformed(Object),
    Url(String),
    Resource(ResourceRequest),
}

pub type RunFuture<'call> =
    Pin<Box<dyn Future<Output = Result<Outcome, ResponsesError>> + Send + 'call>>;

impl Dispatch<'_> {
    // B544's request, URL and resource calls; every reply decoder stays with the leaf
    pub fn run(self) -> Result<Outcome, ResponsesError> {
        let Self { environment, call } = self;
        Ok(match call {
            Call::Transform {
                model,
                input,
                options,
                model_info,
            } => Outcome::Transformed(transform_request(model, input, options, model_info)?),
            Call::CompleteUrl {
                api_base,
                api_version,
            } => Outcome::Url(complete_url(api_base, api_version, environment)?),
            Call::WebsocketUrl { api_base } => Outcome::Url(websocket_url(api_base)?),
            Call::Resource {
                api_base,
                response_id,
                operation,
            } => Outcome::Resource(resource_request(api_base, response_id, operation)?),
            Call::ListInputItems {
                api_base,
                response_id,
                query,
            } => Outcome::Resource(list_input_items(api_base, response_id, query)?),
        })
    }
}

pub fn dispatch(dispatch: Dispatch<'_>) -> RunFuture<'_> {
    Box::pin(ready(dispatch.run()))
}

use std::collections::BTreeMap;

use betterlite_openai::api::container_contracts::config::{
    BaseContainerConfig as _, ContainerRequestContext,
};
use betterlite_openai::containers::urls::join_container_api_base_path;
use serde_json::{Map, json};

use self::calls::{content_reply, headers, reply, request, url};
use self::fixture::call_config;
use self::input::{Globals, Input};
use crate::helpers::HelperResult;

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let shared = call_config(&Globals::default(), &BTreeMap::new())?;
        let litellm_params = Map::new();
        let request_headers = BTreeMap::new();
        let context = ContainerRequestContext {
            litellm_params: &litellm_params,
            headers: &request_headers,
        };
        let supported = if input.supported {
            shared.get_supported_openai_params()
        } else {
            Vec::new()
        };
        let creates: Vec<_> = input
            .creates
            .iter()
            .map(|call| {
                shared.transform_container_create_request(&call.name, &call.params, context)
            })
            .collect();
        Ok(json!({
            "supported": supported,
            "maps": input.maps.iter().map(|call| shared.map_openai_params(&call.params, call.drop_params)).collect::<Vec<_>>(),
            "joins": input.joins.iter().map(|call| join_container_api_base_path(&call.api_base, &call.path_suffix)).collect::<Vec<_>>(),
            "urls": input.urls.iter().map(url).collect::<Result<Vec<_>, _>>()?,
            "creates": creates,
            "requests": input.requests.iter().map(|call| request(&shared, context, call)).collect::<Result<Vec<_>, _>>()?,
            "headers": input.headers.iter().map(headers).collect::<Result<Vec<_>, _>>()?,
            "replies": input.replies.iter().map(|call| reply(&shared, call)).collect::<Vec<_>>(),
            "content_replies": input.content_replies.iter().map(|call| content_reply(&shared, call)).collect::<Result<Vec<_>, _>>()?,
            "errors": input.errors.iter().map(|call| {
                let raised = shared.get_error_class(&call.message, call.status, &call.headers);
                json!({"status_code": raised.status_code, "message": raised.message, "headers": raised.headers})
            }).collect::<Vec<_>>(),
        }))
    }
}

mod calls;
mod fixture;
mod input;
mod tests;

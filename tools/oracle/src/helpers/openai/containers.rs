use std::collections::BTreeMap;

use betterlite_openai::api::container_contracts::config::{
    BaseContainerConfig as _, ContainerRequestContext,
};
use serde_json::{Map, json};

use self::calls::{
    content, content_reply, create, create_reply, error, file_list, headers, join_path, list,
    map_params, metadata_reply, resource, url,
};
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
        Ok(json!({
            "supported": supported,
            "maps": input.maps.iter().map(|call| map_params(&shared, call)).collect::<Vec<_>>(),
            "joins": input.joins.iter().map(join_path).collect::<Vec<_>>(),
            "urls": input.urls.iter().map(url).collect::<Result<Vec<_>, _>>()?,
            "creates": input.creates.iter().map(|call| create(&shared, context, call)).collect::<Vec<_>>(),
            "lists": input.lists.iter().map(|call| list(&shared, context, call)).collect::<Vec<_>>(),
            "file_lists": input.file_lists.iter().map(|call| file_list(&shared, context, call)).collect::<Vec<_>>(),
            "resources": input.resources.iter().map(|call| resource(&shared, context, call)).collect::<Vec<_>>(),
            "contents": input.contents.iter().map(|call| content(&shared, context, call)).collect::<Vec<_>>(),
            "headers": input.headers.iter().map(headers).collect::<Result<Vec<_>, _>>()?,
            "create_replies": input.create_replies.iter().map(|call| create_reply(&shared, call)).collect::<Vec<_>>(),
            "metadata_replies": input.metadata_replies.iter().map(|call| metadata_reply(&shared, call)).collect::<Vec<_>>(),
            "content_replies": input.content_replies.iter().map(|call| content_reply(&shared, call)).collect::<Result<Vec<_>, _>>()?,
            "errors": input.errors.iter().map(|call| error(&shared, call)).collect::<Vec<_>>(),
        }))
    }
}

mod calls;
mod fixture;
mod input;
mod tests;

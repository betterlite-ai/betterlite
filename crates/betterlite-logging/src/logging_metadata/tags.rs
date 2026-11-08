use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use thiserror::Error;

use crate::logging_metadata::params::{LitellmParams, MetadataMap};

pub type ProxyHeaders = IndexMap<String, String>;

#[derive(Clone, Copy, Debug)]
pub struct TagSettings<'input> {
    pub disable_user_agent_tags: bool,
    pub extra_spend_tag_headers: &'input [String],
}

#[derive(Debug, Error)]
pub enum TagError {
    #[error("the metadata field {field}.tags must be a list")]
    TagsType { field: &'static str },
}

#[must_use]
pub fn get_user_agent_tags(
    headers: Option<&ProxyHeaders>,
    disable_add_user_agent: bool,
) -> Vec<Json> {
    let Some(user_agent) = headers.and_then(|present| present.get("user-agent")) else {
        return Vec::new();
    };
    if disable_add_user_agent {
        return Vec::new();
    }
    let prefix = user_agent.split_once('/').map(|(part, _rest)| part);
    prefix
        .into_iter()
        .chain([user_agent.as_str()])
        .map(|part| Json::Text(format!("User-Agent: {part}")))
        .collect()
}

#[must_use]
pub fn get_extra_header_tags(headers: Option<&ProxyHeaders>, names: &[String]) -> Vec<Json> {
    let Some(present) = headers else {
        return Vec::new();
    };
    names
        .iter()
        .filter_map(|name| Some(Json::Text(format!("{name}: {}", present.get(name)?))))
        .collect()
}

pub fn get_request_tags(
    params: &LitellmParams,
    headers: Option<&ProxyHeaders>,
    settings: &TagSettings<'_>,
) -> Result<Vec<Json>, TagError> {
    let mut tags = match declared_tags(params.metadata.as_ref(), "metadata")? {
        Some(tags) => tags,
        None => {
            declared_tags(params.litellm_metadata.as_ref(), "litellm_metadata")?.unwrap_or_default()
        }
    };
    tags.extend(get_user_agent_tags(
        headers,
        settings.disable_user_agent_tags,
    ));
    tags.extend(get_extra_header_tags(
        headers,
        settings.extra_spend_tag_headers,
    ));
    Ok(tags)
}

fn declared_tags(
    metadata: Option<&MetadataMap>,
    field: &'static str,
) -> Result<Option<Vec<Json>>, TagError> {
    let Some(fields) = metadata else {
        return Ok(None);
    };
    match fields.get("tags") {
        None => Ok(None),
        Some(Json::List(items)) => Ok(Some(items.clone())),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            Err(TagError::TagsType { field })
        }
    }
}

mod tests;

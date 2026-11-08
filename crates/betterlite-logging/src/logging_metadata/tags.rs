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
) -> Option<Vec<Json>> {
    if disable_add_user_agent {
        return None;
    }
    let user_agent = headers?.get("user-agent")?;
    let mut tags = Vec::with_capacity(2);
    if let Some((part, _rest)) = user_agent.split_once('/') {
        tags.push(Json::Text(format!("User-Agent: {part}")));
    }
    tags.push(Json::Text(format!("User-Agent: {user_agent}")));
    Some(tags)
}

#[must_use]
pub fn get_extra_header_tags(
    headers: Option<&ProxyHeaders>,
    extra_spend_tag_headers: &[String],
) -> Option<Vec<Json>> {
    if extra_spend_tag_headers.is_empty() {
        return None;
    }
    let present = headers?;
    let mut tags = Vec::new();
    for name in extra_spend_tag_headers {
        if let Some(value) = present.get(name) {
            tags.push(Json::Text(format!("{name}: {value}")));
        }
    }
    if tags.is_empty() {
        return None;
    }
    Some(tags)
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
    if let Some(user_agent_tags) = get_user_agent_tags(headers, settings.disable_user_agent_tags) {
        tags.extend(user_agent_tags);
    }
    if let Some(header_tags) = get_extra_header_tags(headers, settings.extra_spend_tag_headers) {
        tags.extend(header_tags);
    }
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

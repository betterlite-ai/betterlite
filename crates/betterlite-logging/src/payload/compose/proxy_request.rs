use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::logging_metadata::params::LitellmParams;
use crate::logging_metadata::tags::{ProxyHeaders, TagSettings, get_request_tags};
use crate::payload::compose::PayloadComposition;
use crate::payload::compose::inputs::optional_text;
use crate::payload::error::PayloadError;

// litellm_params.proxy_server_request: the headers the tags read and the body's legacy user fallback
#[derive(Default)]
pub(crate) struct ProxyRequest {
    pub headers: Option<ProxyHeaders>,
    pub body_user: Option<String>,
}

impl ProxyRequest {
    pub(crate) fn read(litellm_params: &IndexMap<String, Json>) -> Result<Self, PayloadError> {
        let request = match litellm_params.get("proxy_server_request") {
            None | Some(Json::Null) => return Ok(Self::default()),
            Some(Json::Object(fields)) => fields,
            Some(_) => {
                return Err(PayloadError::FieldType {
                    field: "litellm_params.proxy_server_request",
                });
            }
        };
        let headers = match request.get("headers") {
            None | Some(Json::Null) => None,
            Some(Json::Object(fields)) => Some(header_map(fields)?),
            Some(_) => {
                return Err(PayloadError::FieldType {
                    field: "litellm_params.proxy_server_request.headers",
                });
            }
        };
        let body_user = match request.get("body") {
            None | Some(Json::Null) => None,
            Some(Json::Object(body)) => optional_text(
                body.get("user"),
                "litellm_params.proxy_server_request.body.user",
            )?,
            Some(_) => {
                return Err(PayloadError::FieldType {
                    field: "litellm_params.proxy_server_request.body",
                });
            }
        };
        Ok(Self { headers, body_user })
    }
}

fn header_map(fields: &IndexMap<String, Json>) -> Result<ProxyHeaders, PayloadError> {
    fields
        .iter()
        .map(|(name, value)| match value {
            Json::Text(text) => Ok((name.clone(), text.clone())),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                Err(PayloadError::FieldType {
                    field: "litellm_params.proxy_server_request.headers",
                })
            }
        })
        .collect()
}

pub(crate) fn request_tags(
    params: &LitellmParams,
    proxy: &ProxyRequest,
    composition: &PayloadComposition<'_>,
) -> Result<Vec<Json>, PayloadError> {
    let settings = TagSettings {
        disable_user_agent_tags: composition.settings.tag_config.disable_user_agent_tags,
        extra_spend_tag_headers: &composition.settings.tag_config.extra_spend_tag_headers,
    };
    get_request_tags(params, proxy.headers.as_ref(), &settings)
        .map_err(|source| PayloadError::Tags { source })
}

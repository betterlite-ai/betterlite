use super::request_identity::RequestIdentity;
use super::request_identity::parse::from_payload;
use super::wire::{
    MetadataError,
    payload::{Payload, Response},
};

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct RequestContext {
    pub request_model: String,
    pub response_model: Option<String>,
    pub model_group: Option<String>,
    pub model_id: Option<String>,
    pub api_base: Option<String>,
    pub identity: RequestIdentity,
}

impl RequestContext {
    pub fn from_standard_logging_payload(payload: &Payload) -> Result<Self, MetadataError> {
        let model_group = payload
            .model_group
            .clone()
            .or_else(|| payload.metadata.model_group.clone());
        Ok(Self {
            request_model: model_group
                .clone()
                .or_else(|| payload.model.clone())
                .unwrap_or_default(),
            response_model: match &payload.response {
                Response::Object { model, .. } => model.clone(),
                Response::Text(_) | Response::Items(_) | Response::Null => None,
            },
            model_group,
            model_id: payload
                .model_id
                .clone()
                .or_else(|| payload.metadata.model_info.as_ref()?.id.clone()),
            api_base: payload
                .api_base
                .clone()
                .or_else(|| payload.hidden_params.api_base.clone()),
            identity: from_payload(payload)?,
        })
    }
}

mod tests;

use std::sync::Arc;

use betterlite_bedrock::mantle_inputs::records::MantleGlobals;
use betterlite_bedrock::mantle_inputs::region::resolve_region;
use betterlite_core::cost::context::ProviderInference;
use betterlite_core::cost::recording::{
    GenerateContentConversion, MantleRegionResolver, PortFailure, VertexRegionNormalizer,
};
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_models::decisions::Json;
use betterlite_models::resolution::ProviderIdentity;
use betterlite_vertex::endpoint_target::region::select_region;
use betterlite_vertex::gemini_response::complete::{
    CompleteRequest, RawResponse, transform_response,
};
use betterlite_vertex::gemini_response_parts::projection::CallMode;
use http::HeaderMap;
use indexmap::IndexMap;
use serde_json::json;
use uuid::Uuid;

pub(super) struct Inference;

impl ProviderInference for Inference {
    type Failure = FailureText;

    fn inferred_provider(&self, _model: &str) -> Result<ProviderIdentity, FailureText> {
        Ok(ProviderIdentity::Custom("test".to_owned()))
    }
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
pub(super) struct FailureText(pub(super) String);

pub(super) struct Conversion;

impl GenerateContentConversion for Conversion {
    fn convert(&self, body: &Json, model: &str) -> Result<Json, PortFailure> {
        let bytes = serde_json::to_vec(body).map_err(PortFailure::from)?;
        let headers = HeaderMap::new();
        let raw = RawResponse {
            headers: &headers,
            body: &bytes,
        };
        let request = CompleteRequest {
            model,
            now: 0,
            call_mode: CallMode::LegacyFunction,
        };
        let mut ids = || Uuid::nil();
        let completed = transform_response(&raw, &request, &mut ids).map_err(PortFailure::from)?;
        Ok(json!({
            "model": completed.model,
            "usage": {
                "prompt_tokens": completed.usage.prompt_tokens,
                "completion_tokens": completed.usage.completion_tokens,
                "total_tokens": completed.usage.total_tokens,
            },
        }))
    }
}

pub(super) struct Regions {
    catalog: Arc<ModelCatalog>,
}

impl Regions {
    pub(super) fn new() -> Self {
        Self {
            catalog: BundledCatalog::default().get(),
        }
    }
}

impl VertexRegionNormalizer for Regions {
    fn region(&self, model: &str, configured: Option<&str>) -> String {
        select_region(&self.catalog, model, configured).region
    }
}

pub(super) struct Mantle;

impl MantleRegionResolver for Mantle {
    fn region(&self, litellm_params: &IndexMap<String, Json>) -> Result<String, PortFailure> {
        let globals = MantleGlobals {
            mantle_api_key: None,
            mantle_api_base: None,
            mantle_region: None,
            aws_bearer_token: None,
            aws_region_name: None,
            aws_region: None,
        };
        resolve_region(
            text(litellm_params.get("aws_region_name")),
            text(litellm_params.get("api_base")),
            &globals,
        )
        .map_err(PortFailure::from)
    }
}

fn text(value: Option<&Json>) -> Option<&str> {
    match value {
        Some(Json::Text(text)) => Some(text),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

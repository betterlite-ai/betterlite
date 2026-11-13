use std::collections::BTreeSet;
use std::sync::Arc;

use betterlite_auth::auth_request_models::catalog::{
    CatalogError, ManagedIdDecoders, ManagedIdError, ManagedIdField, ModelCatalog,
    RequestModelResolver,
};
use betterlite_auth::auth_request_models::managed::resolve_model_id_with_router;
use betterlite_auth::auth_request_models::sources::{DispatchContext, RequestSnapshot};
use betterlite_auth::auth_request_models::{
    RequestedModels, push_model_candidates, push_optional_model,
};
use betterlite_auth::resource_hints::managed::{decode_character_id, decode_video_id};
use betterlite_core::api::file_routing_ids::{legacy, routing};
use betterlite_core::api::managed_resource_ids::{is_base64_encoded_unified_id, parse_unified_id};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::auth_identity::AuthContext;

mod tests;

// B1019 owns the legacy prefix; it is not exported, so the reader spells the same literal
const MANAGED_FILE_ID_PREFIX: &str = "litellm_proxy";

#[derive(Debug, Clone)]
pub struct RequestModelStageInput {
    pub context: AuthContext,
    pub route: String,
    pub body: JsonObject,
    pub headers: Option<JsonObject>,
    pub query: Option<JsonObject>,
    pub endpoint_marker: Option<Json>,
    pub path_params: BTreeSet<String>,
    pub team_id: Option<String>,
    pub resolver: Arc<RequestModelResolver>,
}

pub fn resolve(stage: &RequestModelStageInput) -> Result<RequestedModels, CatalogError> {
    let snapshot = RequestSnapshot {
        route: &stage.route,
        body: &stage.body,
        headers: stage.headers.as_ref(),
        query: stage.query.as_ref(),
        dispatch: DispatchContext {
            endpoint_marker: stage.endpoint_marker.as_ref(),
            path_params: &stage.path_params,
        },
        team_id: stage.team_id.as_deref(),
    };
    stage.resolver.resolve(&snapshot)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoreDecoders;

impl ManagedIdDecoders for CoreDecoders {
    fn legacy_file_batch(
        &self,
        resource_id: &str,
        catalog: Option<&dyn ModelCatalog>,
        candidates: &mut Vec<String>,
    ) -> Result<(), ManagedIdError> {
        push_optional_model(candidates, routing::decode_model_from_file_id(resource_id));
        if let Some(unified) = is_base64_encoded_unified_id(resource_id, MANAGED_FILE_ID_PREFIX) {
            push_model_candidates(candidates, legacy::target_model_names(&unified));
            push_optional_model(
                candidates,
                resolve_model_id_with_router(legacy::unified_batch_model_id(&unified), catalog),
            );
        }
        Ok(())
    }

    fn unified(
        &self,
        resource_id: &str,
        catalog: Option<&dyn ModelCatalog>,
        candidates: &mut Vec<String>,
    ) -> Result<(), ManagedIdError> {
        if let Some(parts) = parse_unified_id(resource_id) {
            push_optional_model(
                candidates,
                resolve_model_id_with_router(parts.model_id, catalog),
            );
            push_model_candidates(candidates, parts.target_model_names);
        }
        Ok(())
    }

    fn video_character(
        &self,
        resource_id: &str,
        field: ManagedIdField,
        catalog: Option<&dyn ModelCatalog>,
        candidates: &mut Vec<String>,
    ) -> Result<(), ManagedIdError> {
        let model_id = match field {
            ManagedIdField::Video => decode_video_id(resource_id).model_id,
            ManagedIdField::Character => decode_character_id(resource_id).model_id,
            ManagedIdField::Other => None,
        };
        push_optional_model(candidates, resolve_model_id_with_router(model_id, catalog));
        Ok(())
    }
}

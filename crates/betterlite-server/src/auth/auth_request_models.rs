use std::collections::BTreeSet;
use std::sync::Arc;

use betterlite_auth::auth_request_models::catalog::{
    CatalogError, ManagedIdDecoders, ManagedIdError, ManagedIdFamily, ModelCatalog,
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
    pub endpoint_marker: Option<bool>,
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
            endpoint_marker: stage.endpoint_marker,
            path_params: &stage.path_params,
        },
        team_id: stage.team_id.as_deref(),
    };
    stage.resolver.resolve(&snapshot)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoreDecoders;

impl ManagedIdDecoders for CoreDecoders {
    fn decode_family(
        &self,
        family: ManagedIdFamily,
        resource_id: &str,
        catalog: Option<&dyn ModelCatalog>,
        candidates: &mut Vec<String>,
    ) -> Result<(), ManagedIdError> {
        match family {
            ManagedIdFamily::LegacyFileBatch => {
                push_optional_model(candidates, routing::decode_model_from_file_id(resource_id));
                let Some(unified) =
                    is_base64_encoded_unified_id(resource_id, MANAGED_FILE_ID_PREFIX)
                else {
                    return Ok(());
                };
                push_model_candidates(candidates, legacy::target_model_names(&unified));
                push_optional_model(
                    candidates,
                    resolve_model_id_with_router(legacy::unified_batch_model_id(&unified), catalog),
                );
            }
            ManagedIdFamily::Unified => {
                let Some(parts) = parse_unified_id(resource_id) else {
                    return Ok(());
                };
                push_optional_model(
                    candidates,
                    resolve_model_id_with_router(parts.model_id, catalog),
                );
                push_model_candidates(candidates, parts.target_model_names);
            }
            ManagedIdFamily::Video => push_optional_model(
                candidates,
                resolve_model_id_with_router(decode_video_id(resource_id).model_id, catalog),
            ),
            ManagedIdFamily::Character => push_optional_model(
                candidates,
                resolve_model_id_with_router(decode_character_id(resource_id).model_id, catalog),
            ),
        }
        Ok(())
    }
}

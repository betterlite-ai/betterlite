use betterlite_error::proxy_error_types::ModelAccessObject;

use crate::auth_model_patterns::access_groups::models_from_access_groups;
use crate::auth_model_patterns::decision::{ModelAccessContext, can_object_call_model};
use crate::auth_model_patterns::denial::ModelAccessDecision;
use crate::auth_request_models::RequestedModels;
use crate::auth_team_lookup::lookup::TeamLookup;
use crate::resolved_key::auth::UserAPIKeyAuth;

use super::error::EntityModelAccessError;
use super::grant::{entity_grant, key_grant};
use super::groups::GroupResources;

// can_key_call_model: the key's grant first, then its access groups in the key's place
pub async fn can_key_call_model(
    model: &RequestedModels,
    token: &UserAPIKeyAuth,
    context: &ModelAccessContext<'_>,
    lookup: Option<&TeamLookup>,
) -> Result<(), EntityModelAccessError> {
    let grant = key_grant(token);
    let denial = match can_object_call_model(model, &grant, ModelAccessObject::Key, context, 0) {
        ModelAccessDecision::Allowed => return Ok(()),
        ModelAccessDecision::RecurseDepthExceeded { model } => {
            return Err(EntityModelAccessError::RecurseDepth { model });
        }
        ModelAccessDecision::Denied(denial) => denial,
    };
    let ids = token
        .view
        .token
        .access_group_ids
        .clone()
        .unwrap_or_default();
    if ids.is_empty() {
        return Err(EntityModelAccessError::Denied(denial));
    }
    let groups = GroupResources::load(lookup, &ids).await;
    let models = models_from_access_groups(&ids, Some(&groups));
    if models.is_empty() {
        return Err(EntityModelAccessError::Denied(denial));
    }
    let retry = entity_grant(
        models,
        Some(grant.team_model_aliases),
        Some(grant.key_model_aliases),
        grant.team_id,
    );
    EntityModelAccessError::gate(can_object_call_model(
        model,
        &retry,
        ModelAccessObject::Key,
        context,
        0,
    ))
}

use std::collections::BTreeMap;

use betterlite_db::team_rows::team::LiteLLMTeamTable;
use betterlite_error::proxy_error_types::ModelAccessObject;

use crate::auth_model_lists::lists::dedup_first;
use crate::auth_model_patterns::access_groups::models_from_access_groups;
use crate::auth_model_patterns::decision::{ModelAccessContext, can_object_call_model};
use crate::auth_model_patterns::denial::ModelAccessDecision;
use crate::auth_model_patterns::grant::ModelGrantPattern;
use crate::auth_request_models::RequestedModels;
use crate::auth_team_lookup::lookup::TeamLookup;

use super::error::EntityModelAccessError;
use super::grant::entity_grant;
use super::groups::GroupResources;

// the row, alias maps and group lookup a team grant reads, shared by the first check and its retry
pub struct TeamAccessRequest<'a> {
    pub team: Option<&'a LiteLLMTeamTable>,
    pub team_aliases: &'a BTreeMap<String, String>,
    pub key_aliases: &'a BTreeMap<String, String>,
    pub lookup: Option<&'a TeamLookup>,
}

// can_team_access_model: the team's grant first, then its access groups unioned onto the team models
pub async fn can_team_access_model(
    model: &RequestedModels,
    request: &TeamAccessRequest<'_>,
    context: &ModelAccessContext<'_>,
) -> Result<(), EntityModelAccessError> {
    let grant = team_grant(request);
    match can_object_call_model(model, &grant, ModelAccessObject::Team, context, 0) {
        ModelAccessDecision::Allowed => Ok(()),
        ModelAccessDecision::RecurseDepthExceeded { model: received } => {
            Err(EntityModelAccessError::RecurseDepth { model: received })
        }
        ModelAccessDecision::Denied(denial) => {
            retry_team_groups(
                model,
                request,
                EntityModelAccessError::Denied(denial),
                context,
            )
            .await
        }
    }
}

fn team_grant(request: &TeamAccessRequest<'_>) -> ModelGrantPattern {
    entity_grant(
        request
            .team
            .map_or_else(Vec::new, |row| row.fields.base.models.clone()),
        Some(request.team_aliases.clone()),
        Some(request.key_aliases.clone()),
        request.team.map(|row| row.fields.base.team_id.clone()),
    )
}

// a denied team retries once with its own models followed by the group models, deduplicated in order
async fn retry_team_groups(
    model: &RequestedModels,
    request: &TeamAccessRequest<'_>,
    denied: EntityModelAccessError,
    context: &ModelAccessContext<'_>,
) -> Result<(), EntityModelAccessError> {
    let ids = request
        .team
        .and_then(|row| row.fields.base.access_group_ids.clone())
        .unwrap_or_default();
    if ids.is_empty() {
        return Err(denied);
    }
    let groups = GroupResources::load(request.lookup, &ids).await;
    let models = models_from_access_groups(&ids, Some(&groups));
    if models.is_empty() {
        return Err(denied);
    }
    let mut combined = request
        .team
        .map_or_else(Vec::new, |row| row.fields.base.models.clone());
    combined.extend(models);
    let retry = entity_grant(
        dedup_first(combined),
        Some(request.team_aliases.clone()),
        Some(request.key_aliases.clone()),
        request.team.map(|row| row.fields.base.team_id.clone()),
    );
    EntityModelAccessError::gate(can_object_call_model(
        model,
        &retry,
        ModelAccessObject::Team,
        context,
        0,
    ))
}

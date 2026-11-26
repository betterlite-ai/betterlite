use std::collections::BTreeMap;

use betterlite_db::team_rows::team::LiteLLMTeamTable;
use betterlite_error::proxy_error_types::ModelAccessObject;

use crate::auth_model_lists::lists::dedup_first;
use crate::auth_model_patterns::decision::{ModelAccessContext, can_object_call_model};
use crate::auth_model_patterns::denial::ModelAccessDecision;
use crate::auth_model_patterns::grant::ModelGrantPattern;
use crate::auth_request_models::RequestedModels;
use crate::auth_team_lookup::lookup::TeamLookup;

use super::error::EntityModelAccessError;
use super::grant::team_grant;
use super::groups::{GroupResources, group_models};

// can_team_access_model: the team's grant first, then its access groups unioned onto the team models
pub async fn can_team_access_model(
    model: &RequestedModels,
    team: Option<&LiteLLMTeamTable>,
    aliases: &BTreeMap<String, String>,
    alias_targets: &BTreeMap<String, String>,
    context: &ModelAccessContext<'_>,
    lookup: Option<&TeamLookup>,
) -> Result<(), EntityModelAccessError> {
    let grant = team_grant(team, aliases, alias_targets);
    let denial = match can_object_call_model(model, &grant, ModelAccessObject::Team, context, 0) {
        ModelAccessDecision::Allowed => return Ok(()),
        ModelAccessDecision::RecurseDepthExceeded { model } => {
            return Err(EntityModelAccessError::RecurseDepth { model });
        }
        ModelAccessDecision::Denied(denial) => denial,
    };
    let ids = team
        .and_then(|row| row.fields.base.access_group_ids.clone())
        .unwrap_or_default();
    if ids.is_empty() {
        return Err(EntityModelAccessError::Denied(denial));
    }
    let groups = GroupResources::load(lookup, &ids).await;
    let models = group_models(&groups, &ids);
    if models.is_empty() {
        return Err(EntityModelAccessError::Denied(denial));
    }
    let mut combined = team.map_or_else(Vec::new, |row| row.fields.base.models.clone());
    combined.extend(models);
    let retry = ModelGrantPattern {
        models: dedup_first(combined),
        team_model_aliases: aliases.clone(),
        key_model_aliases: alias_targets.clone(),
        team_id: team.map(|row| row.fields.base.team_id.clone()),
    };
    EntityModelAccessError::gate(can_object_call_model(
        model,
        &retry,
        ModelAccessObject::Team,
        context,
        0,
    ))
}

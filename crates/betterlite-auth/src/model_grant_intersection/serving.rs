use std::collections::BTreeSet;

use crate::model_grant_intersection::error::IntersectionError;
use crate::model_grant_intersection::sources::ServingGroups;

// _model_access_groups_serving_model: the union of the groups serving each requested model.
pub async fn serving_groups(
    models: &[String],
    groups: &dyn ServingGroups,
    team_id: Option<&str>,
) -> Result<BTreeSet<String>, IntersectionError> {
    let mut covering = BTreeSet::new();
    for model in models {
        let served = groups.groups_for(model, team_id).await?;
        covering.extend(served);
    }
    Ok(covering)
}

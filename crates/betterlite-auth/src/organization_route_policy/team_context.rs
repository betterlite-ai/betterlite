use std::error::Error;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

pub const PATCH_TEAM_ROUTE_TEMPLATE: &str = "/team/{team_id}";
const TEAM_UPDATE_ROUTE: &str = "/team/update";

// The caller's team row lookup: LiteLLM passes get_team_object bound to the request's database, and a failed lookup stays an error.
#[async_trait]
pub trait TeamOrganizationLookup {
    type Failure: Error + Send + Sync + 'static;

    async fn team_organization_id(&self, team_id: &str) -> Result<Option<String>, Self::Failure>;
}

// add_team_org_context_to_request_body: the body the route gate reads, a copy that carries the target team's organization_id.
pub async fn add_team_org_context<L: TeamOrganizationLookup + Sync>(
    route: &str,
    body: &JsonObject,
    lookup: &L,
    route_template: Option<&str>,
) -> Result<JsonObject, L::Failure> {
    // the declared type is str | None, so only a non-empty string is the caller's own organization_id
    if body
        .get("organization_id")
        .is_some_and(|value| matches!(value, Json::Text(text) if !text.is_empty()))
    {
        return Ok(body.clone());
    }
    let Some(team_id) = team_id_of(route, body, route_template) else {
        return Ok(body.clone());
    };
    let resolved = lookup.team_organization_id(&team_id).await?;
    let Some(organization_id) = resolved.filter(|organization_id| !organization_id.is_empty())
    else {
        return Ok(body.clone());
    };
    let mut enriched = body.clone();
    enriched.insert("organization_id".to_owned(), Json::Text(organization_id));
    Ok(enriched)
}

// The team id the pinned function reads: body.team_id on /team/update, else the last segment under the exact PATCH template.
fn team_id_of(route: &str, body: &JsonObject, route_template: Option<&str>) -> Option<String> {
    if route == TEAM_UPDATE_ROUTE {
        let Json::Text(team_id) = body.get("team_id")? else {
            return None;
        };
        if team_id.is_empty() {
            return None;
        }
        return Some(team_id.clone());
    }
    if route_template != Some(PATCH_TEAM_ROUTE_TEMPLATE) {
        return None;
    }
    let team_id = route.rsplit('/').next().unwrap_or_default();
    if team_id.is_empty() {
        return None;
    }
    Some(team_id.to_owned())
}

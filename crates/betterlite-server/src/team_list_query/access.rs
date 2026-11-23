use betterlite_auth::auth_user_lookup::lookup::UserLookup;
use betterlite_auth::resolved_key::auth::{UserAPIKeyAuth, user_api_key_has_admin_view};
use betterlite_auth::resolved_key::self_access::allowed_inside_route;
use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::team_rows::team::LiteLLMTeamTable;

use crate::team_access::caller::TeamCaller;

use super::TeamListQueryError;
use super::TeamListRejection;
use super::lookup::{org_admin_org_ids, org_ids_where_admin, request, user_team_ids_from_db};
use super::read::{ALL_TEAMS_SQL, ORG_TEAMS_SQL, find_teams};

// the user_id, org scope and own-team union list_team_v2's query builder then reads
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct V2Access {
    pub user_id: Option<String>,
    pub org_admin_org_ids: Option<Vec<String>>,
    pub own_team_ids: Vec<String>,
}

fn role_text(identity: &TeamCaller) -> String {
    identity
        .user_role
        .map_or_else(|| "None".to_owned(), |role| role.to_string())
}

pub async fn enforce_v2_access(
    lookup: &UserLookup,
    caller: &UserAPIKeyAuth,
    user_id: Option<&str>,
    organization_id: Option<&str>,
) -> Result<V2Access, TeamListQueryError> {
    let identity = TeamCaller::try_from(caller)?;
    if user_api_key_has_admin_view(caller) {
        return Ok(V2Access {
            user_id: user_id.map(str::to_owned),
            org_admin_org_ids: None,
            own_team_ids: Vec::new(),
        });
    }
    let caller_user_id = identity
        .user_id
        .as_deref()
        .filter(|value| !value.is_empty());
    let org_ids = match caller_user_id {
        Some(owner) => org_admin_org_ids(lookup, owner).await?,
        None => None,
    };
    if let (Some(owner), Some(administered)) = (caller_user_id, org_ids) {
        return org_admin_access(lookup, owner, user_id, organization_id, administered).await;
    }
    if !allowed_inside_route(caller, user_id) {
        let role = role_text(&identity);
        return Err(TeamListQueryError::from(TeamListRejection::OtherTeams {
            role,
        }));
    }
    Ok(V2Access {
        user_id: user_id.or(identity.user_id.as_deref()).map(str::to_owned),
        org_admin_org_ids: None,
        own_team_ids: Vec::new(),
    })
}

async fn org_admin_access(
    lookup: &UserLookup,
    caller_user_id: &str,
    user_id: Option<&str>,
    organization_id: Option<&str>,
    administered_org_ids: Vec<String>,
) -> Result<V2Access, TeamListQueryError> {
    if let Some(wanted) = organization_id.filter(|value| !value.is_empty())
        && !administered_org_ids.iter().any(|org_id| org_id == wanted)
    {
        return Err(TeamListQueryError::from(TeamListRejection::OrgScope));
    }
    let is_own_query = match user_id {
        None => true,
        Some(wanted) => wanted == caller_user_id,
    };
    let own_team_ids = if is_own_query {
        user_team_ids_from_db(lookup, caller_user_id).await?
    } else {
        Vec::new()
    };
    Ok(V2Access {
        user_id: if is_own_query {
            None
        } else {
            user_id.map(str::to_owned)
        },
        org_admin_org_ids: Some(administered_org_ids),
        own_team_ids,
    })
}

pub async fn authorize_and_filter_teams(
    lookup: &UserLookup,
    caller: &UserAPIKeyAuth,
    user_id: Option<&str>,
    client: Option<PrismaClient<'_>>,
) -> Result<Vec<LiteLLMTeamTable>, TeamListQueryError> {
    let identity = TeamCaller::try_from(caller)?;
    let is_own_query = matches!(
        (user_id, identity.user_id.as_deref()),
        (Some(wanted), Some(caller_user_id)) if wanted == caller_user_id
    );
    let mut allowed_org_ids: Option<Vec<String>> = None;
    if !user_api_key_has_admin_view(caller) {
        if let Some(caller_user_id) = identity.user_id.as_deref() {
            // the caller row the legacy /team/list authorization reads, with every failure visible
            let found = lookup
                .get_user_object(request(caller_user_id, None))
                .await?;
            let org_ids = found.as_ref().map(org_ids_where_admin).unwrap_or_default();
            allowed_org_ids = (!org_ids.is_empty()).then_some(org_ids);
        }
        if allowed_org_ids.is_none() && !is_own_query {
            let role = role_text(&identity);
            return Err(TeamListQueryError::from(TeamListRejection::OtherTeams {
                role,
            }));
        }
    }
    if let Some(org_ids) = allowed_org_ids.as_deref()
        && !is_own_query
    {
        let teams = find_teams(client, ORG_TEAMS_SQL, &[&org_ids]).await?;
        return Ok(roster_filter(teams, user_id));
    }
    let teams = find_teams(client, ALL_TEAMS_SQL, &[]).await?;
    Ok(roster_filter(teams, user_id))
}

// Prisma cannot filter a JSON roster, so membership is checked after the read
fn roster_filter(teams: Vec<LiteLLMTeamTable>, user_id: Option<&str>) -> Vec<LiteLLMTeamTable> {
    let Some(wanted) = user_id.filter(|value| !value.is_empty()) else {
        return teams;
    };
    teams
        .into_iter()
        .filter(|team| roster_has_member(team, wanted))
        .collect()
}

fn roster_has_member(team: &LiteLLMTeamTable, user_id: &str) -> bool {
    let members = &team.fields.base.members_with_roles;
    !members.is_empty()
        && members
            .iter()
            .any(|member| member.base.user_id.as_deref() == Some(user_id))
}

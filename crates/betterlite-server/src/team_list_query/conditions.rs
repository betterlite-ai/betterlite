use betterlite_auth::auth_user_lookup::lookup::UserLookup;

use crate::key_query_plan::filters::non_empty;

use super::lookup::requested_user_teams;
use super::{TeamListQueryError, TeamListRejection};

// the TeamIdSearchMatch literal: "exact" matches team_id, "prefix" a case-sensitive prefix
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TeamIdSearchMatch {
    #[default]
    Exact,
    Prefix,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeamListColumn {
    TeamId,
    TeamAlias,
    OrganizationId,
}

impl TeamListColumn {
    #[must_use]
    pub const fn wire(self) -> &'static str {
        match self {
            Self::TeamId => "team_id",
            Self::TeamAlias => "team_alias",
            Self::OrganizationId => "organization_id",
        }
    }
}

// one entry of Prisma's team where input
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TeamListWhere {
    Equals(TeamListColumn, String),
    ContainsInsensitive(TeamListColumn, String),
    StartsWith(TeamListColumn, String),
    In(TeamListColumn, Vec<String>),
    MembersHas(String),
    And(Vec<TeamListWhereClause>),
    Or(Vec<TeamListWhereClause>),
}

// a Prisma where object: every entry must hold, and an empty one matches every row
pub type TeamListWhereClause = Vec<TeamListWhere>;

// the query parameters _build_team_list_where_conditions takes besides the access scope
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TeamListFilters {
    pub team_id: Option<String>,
    pub team_alias: Option<String>,
    pub organization_id: Option<String>,
    pub user_id: Option<String>,
    pub search: Option<String>,
    pub search_team_id_match: TeamIdSearchMatch,
    pub use_deleted_table: bool,
}

// _enforce_list_team_v2_access's scope: administered orgs, plus own teams only on an own query
#[derive(Debug, Clone, Copy, Default)]
pub struct TeamListScope<'scope> {
    pub org_admin_org_ids: Option<&'scope [String]>,
    pub own_team_ids: &'scope [String],
}

// the user filter's three outcomes: skip every query, add nothing, or add one condition
enum MembershipFilter {
    SkipQuery,
    NoCondition,
    Condition(TeamListWhere),
}

pub async fn build_where_conditions(
    lookup: &UserLookup,
    filters: &TeamListFilters,
    scope: TeamListScope<'_>,
) -> Result<Option<TeamListWhereClause>, TeamListQueryError> {
    let mut clause = TeamListWhereClause::new();
    push_exact_filters(&mut clause, filters);
    push_search(&mut clause, filters);
    push_organization_scope(&mut clause, filters, scope);
    let Some(user_id) = non_empty(&filters.user_id) else {
        return Ok(Some(clause));
    };
    let user_teams = requested_user_teams(lookup, user_id).await?;
    match membership_filter(filters, user_id, &user_teams)? {
        MembershipFilter::SkipQuery => Ok(None),
        MembershipFilter::NoCondition => Ok(Some(clause)),
        MembershipFilter::Condition(condition) => {
            clause.push(condition);
            Ok(Some(clause))
        }
    }
}

fn push_exact_filters(clause: &mut TeamListWhereClause, filters: &TeamListFilters) {
    if let Some(team_id) = non_empty(&filters.team_id) {
        clause.push(TeamListWhere::Equals(
            TeamListColumn::TeamId,
            team_id.to_owned(),
        ));
    }
    if let Some(team_alias) = non_empty(&filters.team_alias) {
        clause.push(TeamListWhere::ContainsInsensitive(
            TeamListColumn::TeamAlias,
            team_alias.to_owned(),
        ));
    }
}

// the search OR stays one entry so the scope's own AND keeps it from widening the result
fn push_search(clause: &mut TeamListWhereClause, filters: &TeamListFilters) {
    let Some(search) = non_empty(&filters.search) else {
        return;
    };
    let by_team_id = match filters.search_team_id_match {
        TeamIdSearchMatch::Exact => {
            TeamListWhere::Equals(TeamListColumn::TeamId, search.to_owned())
        }
        TeamIdSearchMatch::Prefix => {
            TeamListWhere::StartsWith(TeamListColumn::TeamId, search.to_owned())
        }
    };
    let by_alias = TeamListWhere::ContainsInsensitive(TeamListColumn::TeamAlias, search.to_owned());
    clause.push(TeamListWhere::Or(vec![vec![by_team_id], vec![by_alias]]));
}

fn push_organization_scope(
    clause: &mut TeamListWhereClause,
    filters: &TeamListFilters,
    scope: TeamListScope<'_>,
) {
    if let Some(organization_id) = non_empty(&filters.organization_id) {
        clause.push(TeamListWhere::Equals(
            TeamListColumn::OrganizationId,
            organization_id.to_owned(),
        ));
        return;
    }
    let Some(org_admin_org_ids) = scope.org_admin_org_ids else {
        return;
    };
    if scope.own_team_ids.is_empty() {
        clause.push(TeamListWhere::In(
            TeamListColumn::OrganizationId,
            org_admin_org_ids.to_vec(),
        ));
        return;
    }
    let administered =
        TeamListWhere::In(TeamListColumn::OrganizationId, org_admin_org_ids.to_vec());
    let membership = TeamListWhere::In(TeamListColumn::TeamId, scope.own_team_ids.to_vec());
    clause.push(TeamListWhere::And(vec![vec![TeamListWhere::Or(vec![
        vec![administered],
        vec![membership],
    ])]]));
}

fn membership_filter(
    filters: &TeamListFilters,
    user_id: &str,
    user_teams: &[String],
) -> Result<MembershipFilter, TeamListQueryError> {
    if filters.use_deleted_table {
        return Ok(MembershipFilter::Condition(TeamListWhere::MembersHas(
            user_id.to_owned(),
        )));
    }
    if user_teams.is_empty() {
        return Ok(MembershipFilter::SkipQuery);
    }
    if let Some(team_id) = filters.team_id.as_deref() {
        if !user_teams.iter().any(|member_team| member_team == team_id) {
            return Err(TeamListQueryError::from(TeamListRejection::NotTeamMember {
                team_id: team_id.to_owned(),
            }));
        }
        return Ok(MembershipFilter::NoCondition);
    }
    Ok(MembershipFilter::Condition(TeamListWhere::In(
        TeamListColumn::TeamId,
        user_teams.to_vec(),
    )))
}

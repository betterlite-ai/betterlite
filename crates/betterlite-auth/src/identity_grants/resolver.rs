use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::team_rows::membership::LiteLLMTeamMembership;
use tracing::debug;

use super::loaders::{MembershipLoader, TeamLoader, UserLoader};
use super::records::{
    DegradedRead, GrantDenial, GrantOutcome, IdentityLoadError, ResolvedGrants, ResolvedIdentity,
    UserQuery,
};
use crate::auth_team_lookup::error::TeamLookupError;
use crate::auth_team_lookup::lookup::{TeamLookup, TeamRequest, TeamSource};
use crate::auth_user_lookup::lookup::{UserLookup, UserLookupRequest};
use crate::membership_load::MembershipLookup;

// grants.py's GrantResolver over B736, B737 and B2318; the loaders are injectable for ordering tests
pub struct GrantResolver<
    'readers,
    Users = UserLookup,
    Teams = TeamLookup,
    Members = MembershipLookup,
> {
    pub users: &'readers Users,
    pub teams: &'readers Teams,
    pub memberships: &'readers Members,
}

impl<Users, Teams, Members> GrantResolver<'_, Users, Teams, Members>
where
    Users: UserLoader,
    Teams: TeamLoader,
    Members: MembershipLoader,
{
    // The JWT half: user then membership under the canonical id, with every loader error left to the caller.
    pub async fn resolve_identity(
        &self,
        query: &UserQuery,
        team_id: Option<&str>,
    ) -> Result<ResolvedIdentity, IdentityLoadError> {
        let requested = query.user_id.as_deref();
        let user = self.user(query).await?;
        let effective_user_id = canonical_user_id(requested, user.as_ref()).map(str::to_owned);
        if effective_user_id.as_deref() != requested {
            debug!(
                requested_user_id = requested,
                effective_user_id = effective_user_id.as_deref(),
                "Auth: rebinding user_id to the DB user_id (email/sso match)"
            );
        }
        let membership = self
            .membership(effective_user_id.as_deref(), team_id)
            .await?;
        Ok(ResolvedIdentity {
            user,
            membership,
            effective_user_id,
        })
    }

    // The session refresh: identity, then the live teams array, then the team row itself.
    pub async fn resolve(&self, query: &UserQuery, team_id: Option<&str>) -> GrantOutcome {
        let identity = match self.resolve_identity(query, team_id).await {
            Ok(identity) => identity,
            Err(error) if error.proves_user_gone() => {
                let user_id = query.user_id.clone().unwrap_or_default();
                return GrantOutcome::Denied(GrantDenial::UserGone { user_id });
            }
            Err(error) => return GrantOutcome::Degraded(DegradedRead::Identity(error)),
        };
        let Some(team) = team_id else {
            return GrantOutcome::Resolved(Box::new(ResolvedGrants {
                identity,
                team: None,
            }));
        };
        // a membership row never stands in for a team the live user row no longer lists
        if let Some(user) = identity.user.as_ref()
            && !user.teams.iter().any(|member_of| member_of == team)
        {
            return GrantOutcome::Denied(GrantDenial::NotAMember {
                user_id: user.user_id.clone(),
                team_id: team.to_owned(),
            });
        }
        let request = TeamRequest {
            team_id: team,
            source: TeamSource::CacheThenDatabase,
            team_id_upsert: false,
        };
        match self.teams.load_team(request).await {
            Ok(loaded) => GrantOutcome::Resolved(Box::new(ResolvedGrants {
                identity,
                team: Some(loaded),
            })),
            Err(error) => team_failure(error, team),
        }
    }

    // a falsey user_id never reaches the reader, so its hints are not used either
    async fn user(&self, query: &UserQuery) -> Result<Option<LiteLLMUserTable>, IdentityLoadError> {
        let Some(user_id) = query.user_id.as_deref().filter(|named| !named.is_empty()) else {
            return Ok(None);
        };
        let request = UserLookupRequest {
            user_id: Some(user_id),
            sso_user_id: query.sso_user_id.as_deref(),
            user_email: query.user_email.as_deref(),
            user_id_upsert: query.upsert,
            check_db_only: None,
        };
        self.users
            .load_user(request)
            .await
            .map_err(|error| IdentityLoadError::User(Box::new(error)))
    }

    // both ids must be non-empty; the effective id is the row's own when a fuzzy match rebound it
    async fn membership(
        &self,
        user_id: Option<&str>,
        team_id: Option<&str>,
    ) -> Result<Option<LiteLLMTeamMembership>, IdentityLoadError> {
        match (user_id, team_id) {
            (Some(member), Some(team)) if !member.is_empty() && !team.is_empty() => self
                .memberships
                .load_membership(member, team)
                .await
                .map_err(|error| IdentityLoadError::Membership(Box::new(error))),
            (Some(_) | None, Some(_) | None) => Ok(None),
        }
    }
}

// TeamNotFoundError proves the team is gone; an unreadable team, a 404 included, is a degraded read
fn team_failure(error: TeamLookupError, team_id: &str) -> GrantOutcome {
    match error {
        TeamLookupError::TeamNotFound { .. } => GrantOutcome::Denied(GrantDenial::TeamGone {
            team_id: team_id.to_owned(),
        }),
        other @ (TeamLookupError::NoDatabase
        | TeamLookupError::CacheRead(_)
        | TeamLookupError::NotInCache { .. }
        | TeamLookupError::TeamUnavailable { .. }
        | TeamLookupError::AliasNotFound { .. }
        | TeamLookupError::AliasAmbiguous { .. }
        | TeamLookupError::AliasFailure { .. }
        | TeamLookupError::AccessGroupNotFound { .. }
        | TeamLookupError::AccessGroupFailure { .. }
        | TeamLookupError::AccessPolicyUnavailable { .. }) => {
            GrantOutcome::Degraded(DegradedRead::Team(Box::new(other)))
        }
    }
}

// a row whose id is empty keeps the id the credential named
#[must_use]
pub const fn canonical_user_id<'id>(
    user_id: Option<&'id str>,
    user: Option<&'id LiteLLMUserTable>,
) -> Option<&'id str> {
    match user {
        Some(row) if !row.user_id.is_empty() => Some(row.user_id.as_str()),
        Some(_) | None => user_id,
    }
}

// _UserModelColumn: the whole bare list validates as text, so one other value empties it
#[must_use]
pub fn user_models(user: &LiteLLMUserTable) -> Vec<String> {
    let mut models = Vec::with_capacity(user.models.len());
    for model in &user.models {
        let JsonData::String(name) = model else {
            return Vec::new();
        };
        models.push(name.clone());
    }
    models
}

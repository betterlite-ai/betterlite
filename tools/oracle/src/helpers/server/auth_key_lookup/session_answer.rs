use async_trait::async_trait;
use betterlite_auth::auth_identity::errors::AuthError;
use betterlite_auth::auth_key_lookup::session::refresh_session_token_grants;
use betterlite_auth::auth_team_lookup::error::TeamLookupError;
use betterlite_auth::auth_team_lookup::lookup::TeamRequest;
use betterlite_auth::auth_user_lookup::error::UserLookupError;
use betterlite_auth::auth_user_lookup::lookup::UserLookupRequest;
use betterlite_auth::identity_grants::denial::PublicDenial;
use betterlite_auth::identity_grants::loaders::{MembershipLoader, TeamLoader, UserLoader};
use betterlite_auth::identity_grants::resolver::GrantResolver;
use betterlite_auth::membership_load::error::MembershipLoadError;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_db::team_rows::membership::LiteLLMTeamMembership;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::auth_from_fields;
use super::token_fields;
use super::{SessionInput, UserOutcome};

pub(super) async fn session(input: &SessionInput) -> HelperResult {
    let fixture = session_fixture(input)?;
    let auth = auth_from_fields(&input.auth)?;
    let resolver = GrantResolver {
        users: &fixture.users,
        teams: &fixture.teams,
        memberships: &fixture.members,
    };
    match refresh_session_token_grants(&resolver, &auth).await {
        Ok(refreshed) => Ok(json!({
            "outcome": "ok",
            "denial": Value::Null,
            "fields": token_fields(&refreshed)?,
        })),
        Err(denial) => Ok(json!({
            "outcome": "denied",
            "denial": denial_kind(&denial),
            "fields": Value::Null,
        })),
    }
}

const fn denial_kind(denial: &PublicDenial) -> &'static str {
    match denial {
        PublicDenial::UserGone(_) => "user_gone",
        PublicDenial::TeamGone(_) => "team_gone",
        PublicDenial::NotAMember { .. } => "not_a_member",
    }
}

struct ScriptedUsers {
    row: Option<LiteLLMUserTable>,
    outcome: UserOutcome,
}

#[async_trait]
impl UserLoader for ScriptedUsers {
    async fn load_user(
        &self,
        request: UserLookupRequest<'_>,
    ) -> Result<Option<LiteLLMUserTable>, UserLookupError> {
        match self.outcome {
            UserOutcome::Gone => Err(UserLookupError::NotFound(AuthError::UserNotFound {
                user_id: request.user_id.unwrap_or_default().to_owned(),
            })),
            UserOutcome::Outage => Err(UserLookupError::NoDatabase),
            UserOutcome::Missing => Ok(self.row.clone()),
        }
    }
}

struct ScriptedTeams {
    row: Option<LiteLLMTeamTableCachedObj>,
}

#[async_trait]
impl TeamLoader for ScriptedTeams {
    async fn load_team(
        &self,
        request: TeamRequest<'_>,
    ) -> Result<LiteLLMTeamTableCachedObj, TeamLookupError> {
        self.row
            .clone()
            .ok_or_else(|| TeamLookupError::TeamNotFound {
                team_id: request.team_id.to_owned(),
            })
    }
}

struct ScriptedMembers {
    row: Option<LiteLLMTeamMembership>,
}

#[async_trait]
impl MembershipLoader for ScriptedMembers {
    async fn load_membership(
        &self,
        _user_id: &str,
        _team_id: &str,
    ) -> Result<Option<LiteLLMTeamMembership>, MembershipLoadError> {
        Ok(self.row.clone())
    }
}

struct SessionFixture {
    users: ScriptedUsers,
    teams: ScriptedTeams,
    members: ScriptedMembers,
}

fn session_fixture(input: &SessionInput) -> Result<SessionFixture, Box<HelperError>> {
    Ok(SessionFixture {
        users: ScriptedUsers {
            row: row(input.user.as_ref())?,
            outcome: input.user_outcome,
        },
        teams: ScriptedTeams {
            row: row(input.team.as_ref())?,
        },
        members: ScriptedMembers {
            row: row(input.membership.as_ref())?,
        },
    })
}

fn row<T>(field: Option<&Value>) -> Result<Option<T>, Box<HelperError>>
where
    T: DeserializeOwned,
{
    let Some(row_value) = field else {
        return Ok(None);
    };
    Ok(Some(
        serde_json::from_value(row_value.clone()).or_value_error()?,
    ))
}

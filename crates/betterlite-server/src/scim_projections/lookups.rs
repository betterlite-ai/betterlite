use super::MEMBER_KIND;
use super::error::ProjectionError;
use super::group::project_group;
use super::user::{ScimUserSource, UserProjection, project_user};
use crate::scim_records::group::{Group, Member, MemberKind};
use crate::scim_records::user::{GroupKind, UserGroup};
use betterlite_db::budget_permission_rows::{JsonData, Timestamp};
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::team_repository::repository::{TeamDatabase, TeamRepository};
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use betterlite_db::user_repository::UserRepository;

pub struct ScimLookups<'database> {
    pub store: &'database PostgresStore,
}

// Prisma hands LiteLLM every stored time as UTC, whatever the column type
fn as_utc(times: [&mut Option<Timestamp>; 2]) {
    for time in times.into_iter().flatten() {
        *time = Timestamp::Aware(time.utc().fixed_offset());
    }
}

// a team deleted after the user joined it is skipped, while the user row itself is never filtered
async fn user_groups(
    teams: &TeamRepository<'_>,
    team_ids: &[String],
) -> Result<Vec<UserGroup>, ProjectionError> {
    let mut groups = Vec::new();
    for team_id in team_ids {
        let Some(team) = teams.table().find_unique(team_id).await? else {
            continue;
        };
        let display = match team.fields.get("team_alias") {
            Some(JsonData::String(alias)) => Some(alias.clone()),
            Some(
                JsonData::Null
                | JsonData::Bool(_)
                | JsonData::Number(_)
                | JsonData::Array(_)
                | JsonData::Object(_),
            )
            | None => None,
        };
        groups.push(UserGroup {
            value: team_id.clone(),
            display,
            kind: GroupKind::default(),
        });
    }
    Ok(groups)
}

impl<'database> ScimLookups<'database> {
    pub const fn connected(&self) -> Result<&'database PostgresStore, ProjectionError> {
        if self.store.pool.is_none() {
            return Err(ProjectionError::NoDatabase);
        }
        Ok(self.store)
    }

    fn team_repository(&self) -> Result<TeamRepository<'database>, ProjectionError> {
        let store = self.connected()?;
        Ok(TeamRepository {
            database: TeamDatabase {
                db: store,
                writer_db: store,
            },
            use_writer: false,
        })
    }

    async fn find_user(&self, user_id: &str) -> Result<Option<LiteLLMUserTable>, ProjectionError> {
        let client = self.connected()?.session().await?;
        let users = UserRepository {
            db: Some(&client),
            writer_db: None,
            use_writer: false,
        };
        let mut found = users.find_by_id(user_id).await?;
        if let Some(user) = &mut found {
            as_utc([&mut user.created_at, &mut user.updated_at]);
        }
        Ok(found)
    }

    pub async fn check_user_exists(
        &self,
        user_id: &str,
    ) -> Result<LiteLLMUserTable, ProjectionError> {
        let found = self.find_user(user_id).await?;
        found.ok_or_else(|| ProjectionError::UserNotFound {
            user_id: user_id.to_owned(),
        })
    }

    // the team row is validated as a whole here, where LiteLLM hands back the unvalidated Prisma row
    pub async fn check_team_exists(
        &self,
        team_id: &str,
    ) -> Result<LiteLLMTeamTable, ProjectionError> {
        let teams = self.team_repository()?;
        let Some(mut team) = teams.find_by_id(team_id).await? else {
            return Err(ProjectionError::GroupNotFound {
                team_id: team_id.to_owned(),
            });
        };
        as_utc([&mut team.fields.created_at, &mut team.fields.updated_at]);
        Ok(team)
    }

    pub async fn project_stored_user(
        &self,
        user: &ScimUserSource<'_>,
    ) -> Result<UserProjection, ProjectionError> {
        let teams = self.team_repository()?;
        let groups = user_groups(&teams, user.teams).await?;
        project_user(user, groups)
    }

    pub fn project_stored_group(&self, team: &LiteLLMTeamTable) -> Result<Group, ProjectionError> {
        self.connected()?;
        project_group(team)
    }

    // a roster id with no user row is left out, while the group projection keeps it
    pub async fn get_team_members_display(
        &self,
        member_ids: &[String],
    ) -> Result<Vec<Member>, ProjectionError> {
        self.connected()?;
        let mut members = Vec::new();
        for member_id in member_ids {
            if let Some(user) = self.find_user(member_id).await? {
                let display = user
                    .user_email
                    .as_deref()
                    .filter(|email| !email.is_empty())
                    .unwrap_or(&user.user_id);
                members.push(Member {
                    value: user.user_id.clone(),
                    display: Some(display.to_owned()),
                    kind: MemberKind(Some(MEMBER_KIND.to_owned())),
                });
            }
        }
        Ok(members)
    }
}

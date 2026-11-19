use std::collections::BTreeSet;

use super::ScimFixtureError;
use super::ScimSeed;
use super::model::{ScimKeyRow, ScimTeamRow, ScimUserRow};
use crate::finops::model::secret_eq;
use crate::seed::FixtureSeed;

impl ScimSeed {
    pub fn validate(&self, fixture: &FixtureSeed) -> Result<(), ScimFixtureError> {
        referenced_rows(&self.users, &self.teams, fixture)?;
        key_rows(&self.keys, fixture)?;
        user_roles(&self.users, fixture)
    }
}

fn referenced_rows(
    users: &[ScimUserRow],
    teams: &[ScimTeamRow],
    fixture: &FixtureSeed,
) -> Result<(), ScimFixtureError> {
    let mut seen_users = BTreeSet::new();
    for (row, user) in users.iter().enumerate() {
        if !fixture
            .users
            .iter()
            .any(|seeded| seeded.user_id == user.user_id)
        {
            return Err(ScimFixtureError::MissingUser {
                row,
                user_id: user.user_id.clone(),
            });
        }
        if !seen_users.insert(user.user_id.as_str()) {
            return Err(ScimFixtureError::Duplicate { row, kind: "user" });
        }
    }
    let mut seen_teams = BTreeSet::new();
    for (row, team) in teams.iter().enumerate() {
        if !fixture
            .teams
            .iter()
            .any(|seeded| seeded.team_id == team.team_id)
        {
            return Err(ScimFixtureError::MissingTeam {
                row,
                team_id: team.team_id.clone(),
            });
        }
        if !seen_teams.insert(team.team_id.as_str()) {
            return Err(ScimFixtureError::Duplicate { row, kind: "team" });
        }
    }
    Ok(())
}

fn key_rows(keys: &[ScimKeyRow], fixture: &FixtureSeed) -> Result<(), ScimFixtureError> {
    let mut seen = BTreeSet::new();
    for (row, key) in keys.iter().enumerate() {
        let Some(target) = fixture
            .keys
            .iter()
            .position(|seeded| secret_eq(&seeded.token, &key.token))
        else {
            return Err(ScimFixtureError::UnknownKey { row });
        };
        if !seen.insert(target) {
            return Err(ScimFixtureError::Duplicate { row, kind: "key" });
        }
    }
    Ok(())
}

fn user_roles(users: &[ScimUserRow], fixture: &FixtureSeed) -> Result<(), ScimFixtureError> {
    for (row, user) in users.iter().enumerate() {
        let Some(role) = user.user_role else {
            continue;
        };
        let disagrees = fixture.keys.iter().any(|key| {
            key.user_id == user.user_id && key.role.is_some_and(|key_role| key_role != role)
        });
        if disagrees {
            return Err(ScimFixtureError::ConflictingUserRole {
                row,
                user_id: user.user_id.clone(),
            });
        }
    }
    Ok(())
}

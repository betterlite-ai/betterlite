use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::error::FixtureError;

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DatabaseFixture {
    #[serde(default)]
    pub seed: FixtureSeed,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureSeed {
    #[serde(default)]
    pub users: Vec<UserRow>,
    #[serde(default)]
    pub keys: Vec<KeyRow>,
    #[serde(default)]
    pub teams: Vec<TeamRow>,
    #[serde(default)]
    pub memberships: Vec<Membership>,
    #[serde(default)]
    pub budgets: Vec<BudgetRow>,
    #[serde(default)]
    pub jwt_mappings: Vec<JwtMapping>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UserRow {
    pub user_id: String,
    #[serde(default)]
    pub user_email: Option<String>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KeyRow {
    pub token: String,
    pub user_id: String,
    #[serde(default)]
    pub team_id: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
}

impl fmt::Debug for KeyRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyRow")
            .field("token", &"[redacted]")
            .field("user_id", &self.user_id)
            .field("team_id", &self.team_id)
            .field("role", &self.role)
            .finish()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TeamRow {
    pub team_id: String,
    #[serde(default)]
    pub team_alias: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Membership {
    pub user_id: String,
    pub team_id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BudgetRow {
    pub budget_id: String,
    pub user_id: String,
    #[serde(default)]
    pub max_budget: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct JwtMapping {
    pub subject: String,
    pub user_id: String,
}

impl FixtureSeed {
    pub fn validate(&self) -> Result<(), FixtureError> {
        unique(self.users.iter().map(|row| row.user_id.as_str()), "user")?;
        unique(self.teams.iter().map(|row| row.team_id.as_str()), "team")?;
        unique(
            self.budgets.iter().map(|row| row.budget_id.as_str()),
            "budget",
        )?;
        unique(
            self.jwt_mappings.iter().map(|row| row.subject.as_str()),
            "JWT mapping",
        )?;
        let users = self
            .users
            .iter()
            .map(|row| row.user_id.as_str())
            .collect::<BTreeSet<_>>();
        let teams = self
            .teams
            .iter()
            .map(|row| row.team_id.as_str())
            .collect::<BTreeSet<_>>();
        validate_keys_and_memberships(&users, &teams, self)?;
        validate_budgets_and_mappings(&users, self)?;
        Ok(())
    }

    #[must_use]
    pub fn key_hash(token: &str) -> String {
        let mut hash = Sha256::new();
        hash.update(token.as_bytes());
        hex::encode(hash.finalize())
    }
}

fn validate_keys_and_memberships(
    users: &BTreeSet<&str>,
    teams: &BTreeSet<&str>,
    seed: &FixtureSeed,
) -> Result<(), FixtureError> {
    for key in &seed.keys {
        if !users.contains(key.user_id.as_str()) {
            return Err(FixtureError::Seed(format!(
                "key references missing user {}",
                key.user_id
            )));
        }
        if let Some(team_id) = &key.team_id
            && !teams.contains(team_id.as_str())
        {
            return Err(FixtureError::Seed(format!(
                "key references missing team {team_id}"
            )));
        }
    }
    for membership in &seed.memberships {
        if !users.contains(membership.user_id.as_str())
            || !teams.contains(membership.team_id.as_str())
        {
            return Err(FixtureError::Seed(
                "membership references a missing principal".to_owned(),
            ));
        }
    }
    Ok(())
}

fn validate_budgets_and_mappings(
    users: &BTreeSet<&str>,
    seed: &FixtureSeed,
) -> Result<(), FixtureError> {
    for budget in &seed.budgets {
        if !users.contains(budget.user_id.as_str()) {
            return Err(FixtureError::Seed(format!(
                "budget references missing user {}",
                budget.user_id
            )));
        }
    }
    for mapping in &seed.jwt_mappings {
        if !users.contains(mapping.user_id.as_str()) {
            return Err(FixtureError::Seed(format!(
                "JWT mapping references missing user {}",
                mapping.user_id
            )));
        }
    }
    Ok(())
}

fn unique<'item>(values: impl Iterator<Item = &'item str>, kind: &str) -> Result<(), FixtureError> {
    let mut seen = BTreeSet::new();
    for value in values {
        if !seen.insert(value) {
            return Err(FixtureError::Seed(format!(
                "duplicate {kind} identity {value}"
            )));
        }
    }
    Ok(())
}

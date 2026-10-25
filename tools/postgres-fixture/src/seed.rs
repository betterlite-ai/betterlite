use std::collections::BTreeSet;
use std::fmt;

use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use sha2::{Digest as _, Sha256};

use crate::error::FixtureError;

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatabaseFixture {
    #[serde(default)]
    pub seed: FixtureSeed,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserRow {
    pub user_id: String,
    #[serde(default)]
    pub user_email: Option<String>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyRow {
    pub token: SecretString,
    pub user_id: String,
    #[serde(default)]
    pub team_id: Option<String>,
    #[serde(default)]
    pub role: Option<CallerRole>,
}

impl PartialEq for KeyRow {
    fn eq(&self, other: &Self) -> bool {
        self.token.expose_secret() == other.token.expose_secret()
            && self.user_id == other.user_id
            && self.team_id == other.team_id
            && self.role == other.role
    }
}

impl Eq for KeyRow {}

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

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TeamRow {
    pub team_id: String,
    #[serde(default)]
    pub team_alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Membership {
    pub user_id: String,
    pub team_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BudgetRow {
    pub budget_id: String,
    pub user_id: String,
    #[serde(default)]
    pub max_budget: Option<Decimal>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JwtMapping {
    pub subject: String,
    pub user_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallerRole {
    ProxyAdmin,
    ProxyAdminViewer,
    OrgAdmin,
    InternalUser,
    InternalUserViewer,
    Team,
    Customer,
}

impl CallerRole {
    #[must_use]
    pub const fn as_wire(self) -> &'static str {
        match self {
            Self::ProxyAdmin => "proxy_admin",
            Self::ProxyAdminViewer => "proxy_admin_viewer",
            Self::OrgAdmin => "org_admin",
            Self::InternalUser => "internal_user",
            Self::InternalUserViewer => "internal_user_viewer",
            Self::Team => "team",
            Self::Customer => "customer",
        }
    }
}

impl FixtureSeed {
    pub fn validate(&self) -> Result<(), FixtureError> {
        unique(self.users.iter().map(|row| row.user_id.as_str()), "user")?;
        unique(self.keys.iter().map(|row| row.token.expose_secret()), "key")?;
        unique(self.teams.iter().map(|row| row.team_id.as_str()), "team")?;
        unique_memberships(&self.memberships)?;
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
        validate_references(&users, &teams, self)?;
        Ok(())
    }

    #[must_use]
    pub fn key_hash(token: &str) -> String {
        let mut hash = Sha256::new();
        hash.update(token.as_bytes());
        hex::encode(hash.finalize())
    }
}

fn validate_references(
    users: &BTreeSet<&str>,
    teams: &BTreeSet<&str>,
    seed: &FixtureSeed,
) -> Result<(), FixtureError> {
    for key in &seed.keys {
        ensure_reference(users, &key.user_id, "key")?;
        if let Some(team_id) = &key.team_id
            && !teams.contains(team_id.as_str())
        {
            return Err(FixtureError::Seed(format!(
                "key references missing team {team_id}"
            )));
        }
    }
    for membership in &seed.memberships {
        ensure_reference(users, &membership.user_id, "membership")?;
        ensure_reference(teams, &membership.team_id, "membership")?;
    }
    for budget in &seed.budgets {
        ensure_reference(users, &budget.user_id, "budget")?;
    }
    for mapping in &seed.jwt_mappings {
        ensure_reference(users, &mapping.user_id, "JWT mapping")?;
    }
    Ok(())
}

fn ensure_reference(
    principals: &BTreeSet<&str>,
    identity: &str,
    relation: &str,
) -> Result<(), FixtureError> {
    if !principals.contains(identity) {
        return Err(FixtureError::Seed(format!(
            "{relation} references missing principal {identity}"
        )));
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

fn unique_memberships(memberships: &[Membership]) -> Result<(), FixtureError> {
    let mut seen = BTreeSet::new();
    for membership in memberships {
        if !seen.insert((membership.user_id.as_str(), membership.team_id.as_str())) {
            return Err(FixtureError::Seed(format!(
                "duplicate membership identity {}/{}",
                membership.user_id, membership.team_id
            )));
        }
    }
    Ok(())
}

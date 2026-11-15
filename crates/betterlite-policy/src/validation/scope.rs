use std::fmt::Display;
use std::future::Future;

use super::PolicyValidator;
use super::lookup::{AliasDatabase, ModelRouter};
use super::types::{PolicyValidationError, PolicyValidationErrorType};

struct ScopeKind {
    error_type: PolicyValidationErrorType,
    field: &'static str,
    label: &'static str,
    title: &'static str,
}

const TEAMS: ScopeKind = ScopeKind {
    error_type: PolicyValidationErrorType::InvalidTeam,
    field: "teams",
    label: "team",
    title: "Team",
};
const KEYS: ScopeKind = ScopeKind {
    error_type: PolicyValidationErrorType::InvalidKey,
    field: "keys",
    label: "key",
    title: "Key",
};
const MODELS: ScopeKind = ScopeKind {
    error_type: PolicyValidationErrorType::InvalidModel,
    field: "models",
    label: "model",
    title: "Model",
};

impl ScopeKind {
    fn error(&self, policy_name: &str, value: &str) -> PolicyValidationError {
        let Self { label, title, .. } = self;
        PolicyValidationError {
            policy_name: policy_name.to_owned(),
            error_type: self.error_type,
            message: format!(
                "{title} '{value}' does not exist. Reference an existing {label} or use a wildcard pattern (e.g. '{value}*') to match by prefix."
            ),
            field: Some(self.field.to_owned()),
            value: Some(value.to_owned()),
        }
    }
}

async fn assume_exists<Lookup, Error>(lookup: Lookup, label: &str, alias: &str) -> bool
where
    Lookup: Future<Output = Result<bool, Error>>,
    Error: Display,
{
    match lookup.await {
        Ok(exists) => exists,
        Err(error) => {
            tracing::warn!("Could not check {label} '{alias}': {error}");
            true
        }
    }
}

impl<Guardrails: Sync, Database: AliasDatabase + Sync, Router: ModelRouter + Sync>
    PolicyValidator<Guardrails, Database, Router>
{
    pub async fn check_team_alias_exists(&self, team_alias: &str) -> bool {
        let Some(database) = &self.database else {
            return true;
        };
        assume_exists(
            database.team_alias_exists(team_alias),
            "team alias",
            team_alias,
        )
        .await
    }

    pub async fn check_key_alias_exists(&self, key_alias: &str) -> bool {
        let Some(database) = &self.database else {
            return true;
        };
        assume_exists(database.key_alias_exists(key_alias), "key alias", key_alias).await
    }

    #[must_use]
    pub fn check_model_exists(&self, model: &str) -> bool {
        let Some(router) = &self.router else {
            return true;
        };
        let known = match router.has_model_name(model) {
            Ok(true) => Ok(true),
            Ok(false) => router.has_pattern_deployments(model),
            Err(error) => Err(error),
        };
        known.unwrap_or_else(|error| {
            tracing::warn!("Could not check model '{model}': {error}");
            true
        })
    }

    pub async fn find_invalid_scope_entries(
        &self,
        policy_name: &str,
        teams: &[String],
        keys: &[String],
        models: &[String],
    ) -> Vec<PolicyValidationError> {
        let mut errors = Vec::new();
        // RouteChecks._is_wildcard_pattern: only a trailing star makes a pattern
        for team in teams.iter().filter(|entry| !entry.ends_with('*')) {
            if !self.check_team_alias_exists(team).await {
                errors.push(TEAMS.error(policy_name, team));
            }
        }
        for key in keys.iter().filter(|entry| !entry.ends_with('*')) {
            if !self.check_key_alias_exists(key).await {
                errors.push(KEYS.error(policy_name, key));
            }
        }
        for model in models.iter().filter(|entry| !entry.ends_with('*')) {
            if !self.check_model_exists(model) {
                errors.push(MODELS.error(policy_name, model));
            }
        }
        errors
    }
}

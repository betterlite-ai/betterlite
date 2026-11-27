use async_trait::async_trait;
use betterlite_auth::auth_key_lookup::error::KeyLookupError;
use betterlite_auth::auth_team_lookup::error::TeamLookupError;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use thiserror::Error;

use super::metadata::{
    USER_API_KEY, USER_API_KEY_ALIAS, USER_API_KEY_ORG_ID, USER_API_KEY_PROJECT_ID,
    USER_API_KEY_TEAM_ALIAS, USER_API_KEY_TEAM_ID, USER_API_KEY_USER_ID, missing, optional_text,
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyFacts {
    pub key_alias: Option<String>,
    pub user_id: Option<String>,
    pub team_id: Option<String>,
    pub org_id: Option<String>,
    pub project_id: Option<String>,
}

#[derive(Debug, Error)]
pub enum IdentityLookupError {
    #[error(transparent)]
    Key(#[from] KeyLookupError),
    #[error(transparent)]
    Team(#[from] TeamLookupError),
}

// get_key_object of B738 and get_team_object of B737 behind one port; the wiring binds it.
#[async_trait]
pub trait FailureIdentityLookup: Send + Sync {
    async fn key_facts(&self, hashed_token: &str) -> Result<KeyFacts, IdentityLookupError>;

    async fn team_alias(&self, team_id: &str) -> Result<Option<String>, IdentityLookupError>;
}

// _enrich_failure_metadata_with_key_info: null-only fills, an independent team lookup, and debug-only errors.
pub async fn enrich_failure_metadata(
    metadata: &mut JsonObject,
    lookup: Option<&dyn FailureIdentityLookup>,
    resolve_missing_key_identity: bool,
) {
    let Some(token) = truthy_text(metadata.get(USER_API_KEY)).map(str::to_owned) else {
        return;
    };
    let Some(resolver) = lookup else {
        return;
    };
    if resolve_missing_key_identity && missing(metadata.get(USER_API_KEY_ALIAS)) {
        match resolver.key_facts(&token).await {
            Ok(facts) => fill_missing_identity(metadata, facts),
            Err(error) => tracing::debug!(%error, "failure metadata key enrichment failed"),
        }
    }
    let Some(team_id) = truthy_text(metadata.get(USER_API_KEY_TEAM_ID)).map(str::to_owned) else {
        return;
    };
    if !missing(metadata.get(USER_API_KEY_TEAM_ALIAS)) {
        return;
    }
    match resolver.team_alias(&team_id).await {
        Ok(Some(alias)) => {
            metadata.insert(USER_API_KEY_TEAM_ALIAS.to_owned(), Json::Text(alias));
        }
        Ok(None) => {}
        Err(error) => tracing::debug!(%error, "failure metadata team alias enrichment failed"),
    }
}

fn fill_missing_identity(metadata: &mut JsonObject, facts: KeyFacts) {
    for (key, value) in [
        (USER_API_KEY_ALIAS, facts.key_alias),
        (USER_API_KEY_USER_ID, facts.user_id),
        (USER_API_KEY_TEAM_ID, facts.team_id),
        (USER_API_KEY_ORG_ID, facts.org_id),
        (USER_API_KEY_PROJECT_ID, facts.project_id),
    ] {
        if missing(metadata.get(key)) {
            metadata.insert(key.to_owned(), optional_text(value));
        }
    }
}

fn truthy_text(value: Option<&Json>) -> Option<&str> {
    match value {
        Some(Json::Text(text)) if !text.is_empty() => Some(text),
        Some(
            Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::Text(_)
            | Json::List(_)
            | Json::Object(_),
        )
        | None => None,
    }
}

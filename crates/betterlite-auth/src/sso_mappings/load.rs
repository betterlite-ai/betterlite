use betterlite_config::sso_mappings::SsoMappingSources;
use serde::Deserialize;
use serde::de::DeserializeOwned;

use super::repository::SsoSettingsRows;
use super::roles::{RoleMappingError, decode_role_mapping_literal};
use crate::sso_types::records::{RoleMappings, TeamMappings};

// LiteLLM reads only these keys out of sso_settings, so any other stored key cannot fail the read
#[derive(Deserialize)]
struct StoredRoleMappings {
    role_mappings: Option<RoleMappings>,
}

#[derive(Deserialize)]
struct StoredTeamMappings {
    team_mappings: Option<TeamMappings>,
}

pub async fn load_role_mappings<S>(
    rows: &S,
    sources: &SsoMappingSources,
) -> Result<Option<RoleMappings>, RoleMappingError>
where
    S: SsoSettingsRows,
{
    let stored = stored_settings_text(rows)
        .await
        .and_then(|text| decode_stored::<StoredRoleMappings>(&text))
        .and_then(|settings| settings.role_mappings);
    let Some(literal) = sources.role_mappings_roles.as_deref() else {
        return Ok(stored);
    };
    let overridden = decode_role_mapping_literal(
        literal.as_bytes(),
        sources.role_mappings_group_claim.as_deref(),
        sources.role_mappings_default_role.as_deref(),
    )?;
    Ok(overridden.or(stored))
}

pub async fn load_team_mappings<S>(rows: &S) -> Option<TeamMappings>
where
    S: SsoSettingsRows,
{
    let text = stored_settings_text(rows).await?;
    let settings = decode_stored::<StoredTeamMappings>(&text)?;
    settings.team_mappings
}

async fn stored_settings_text<S>(rows: &S) -> Option<String>
where
    S: SsoSettingsRows,
{
    match rows.stored_sso_settings().await {
        Ok(text) => text,
        Err(error) => {
            tracing::debug!(
                error = %error,
                "the SSO settings row could not be read; continuing without it",
            );
            None
        }
    }
}

fn decode_stored<T>(text: &str) -> Option<T>
where
    T: DeserializeOwned,
{
    match serde_json::from_str(text) {
        Ok(settings) => Some(settings),
        Err(error) => {
            tracing::debug!(
                error = %error,
                "the SSO settings row did not decode; continuing without it",
            );
            None
        }
    }
}

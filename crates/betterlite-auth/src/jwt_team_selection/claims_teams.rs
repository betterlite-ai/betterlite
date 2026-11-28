use std::collections::BTreeSet;

use betterlite_db::team_rows::team::LiteLLMTeamTable;

use crate::auth_team_lookup::alias::get_team_object_by_alias;
use crate::auth_team_lookup::error::TeamLookupError;
use crate::auth_team_lookup::lookup::{TeamLookup, TeamRequest, TeamSource};
use crate::auth_team_lookup::team::get_team_object;
use crate::jwt_claims::claims::JwtClaims;
use crate::jwt_claims::teams::TeamClaims as _;

use super::error::TeamSelectionError;
use super::settings::SelectionSettings;

// find_and_validate_specific_team_id: the individual claim first, then the alias, then the required-team refusal
pub async fn find_and_validate_specific_team_id(
    claims: &JwtClaims<'_>,
    lookup: &TeamLookup,
    settings: &SelectionSettings,
    team_id_upsert: Option<bool>,
) -> Result<(Option<String>, Option<LiteLLMTeamTable>), TeamSelectionError> {
    let mut individual_team_id = claims.team_id(None)?;
    let team_alias = claims.team_alias(None)?;
    // a configured team_id_default must not mask a real alias claim the token carries
    if team_alias.is_some() && individual_team_id.is_some() && claims.all_team_ids()?.is_empty() {
        individual_team_id = None;
    }
    if let Some(team_id) = individual_team_id.as_deref() {
        let upsert = team_id_upsert.unwrap_or(settings.teams.team_id_upsert);
        let request = TeamRequest {
            team_id,
            source: TeamSource::CacheThenDatabase,
            team_id_upsert: upsert,
        };
        match get_team_object(lookup, request).await {
            Ok(team) => return Ok((Some(team_id.to_owned()), Some(team.team))),
            // only an absent row defers to the fallback; every other failure is the caller's answer
            Err(error) if not_found(&error) && settings.routing.team_claim_fallback => {
                return Ok((None, None));
            }
            Err(error) => return Err(TeamSelectionError::Lookup(Box::new(error))),
        }
    }
    if let Some(alias) = team_alias.as_deref() {
        let team = get_team_object_by_alias(lookup, alias)
            .await
            .map_err(|error| TeamSelectionError::Lookup(Box::new(error)))?;
        return Ok((Some(team.team.fields.base.team_id.clone()), Some(team.team)));
    }
    if settings.requires_team() {
        return Err(missing_team(settings));
    }
    Ok((None, None))
}

// get_all_team_ids: the plural groups only, deduplicated
pub fn get_all_team_ids(claims: &JwtClaims<'_>) -> Result<BTreeSet<String>, TeamSelectionError> {
    Ok(claims.team_ids()?.into_iter().collect())
}

fn not_found(error: &TeamLookupError) -> bool {
    error
        .http()
        .is_some_and(|(status_code, _detail)| status_code == 404)
}

// the plain exception that names both claim fields, with the operator hint for an unusable field path
fn missing_team(settings: &SelectionSettings) -> TeamSelectionError {
    let team_id_field = settings.teams.team_id_jwt_field.as_deref();
    let team_alias_field = settings.teams.team_alias_jwt_field.as_deref();
    let hint = team_id_field.and_then(index_hint).unwrap_or_default();
    TeamSelectionError::no_team_in_token(team_id_field, team_alias_field, &hint)
}

// the dot-notation and bracket-notation hints LiteLLM gives for a field path that indexes a list
fn index_hint(field: &str) -> Option<String> {
    if let Some((base, index)) = field.rsplit_once('.') {
        if !index.is_empty() && index.chars().all(|character| character.is_ascii_digit()) {
            return Some(format!(
                " Hint: dot-notation array indexing (e.g. '{field}') is not supported. Use '{base}' instead — LiteLLM automatically uses the first element when the field value is a list."
            ));
        }
    }
    let body = field.strip_suffix(']')?;
    let (base, index) = body.split_once('[')?;
    let identifier = !base.is_empty()
        && base
            .chars()
            .all(|character| character.is_alphanumeric() || character == '_');
    if identifier && !index.is_empty() && index.chars().all(|character| character.is_ascii_digit())
    {
        return Some(format!(
            " Hint: array indexing (e.g. '{field}') is not supported in team_id_jwt_field. Use '{base}' instead — LiteLLM automatically uses the first element when the field value is a list."
        ));
    }
    None
}

use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_db::team_rows::decoder::TeamRowError;
use betterlite_db::team_rows::team::LiteLLMTeamTable;

use crate::auth_model_patterns::grant::{ModelGrantPattern, key_model_aliases, resolve_key_models};
use crate::resolved_key::auth::UserAPIKeyAuth;

use super::error::EntityModelAccessError;

// token.models and token.team_models are list columns; an entry that is not a name is not a grant
#[must_use]
pub fn text_models(models: &[JsonData]) -> Vec<String> {
    let mut names = Vec::with_capacity(models.len());
    for model in models {
        let JsonData::String(name) = model else {
            continue;
        };
        names.push(name.clone());
    }
    names
}

// one shape for every entity grant: its models, the aliases that apply and the team it resolves through
#[must_use]
pub fn entity_grant(
    models: Vec<String>,
    team_aliases: Option<BTreeMap<String, String>>,
    key_aliases: Option<BTreeMap<String, String>>,
    team_id: Option<String>,
) -> ModelGrantPattern {
    ModelGrantPattern {
        models,
        team_model_aliases: team_aliases.unwrap_or_default(),
        key_model_aliases: key_aliases.unwrap_or_default(),
        team_id,
    }
}

// _resolve_key_models_for_auth_check, then key_model_aliases_for_auth_check over the same token
#[must_use]
pub fn key_grant(token: &UserAPIKeyAuth) -> ModelGrantPattern {
    let fields = &token.view.token;
    let models = text_models(&fields.models);
    let team_models = text_models(&token.view.team_models);
    entity_grant(
        resolve_key_models(&models, &team_models, fields.team_id.as_deref()),
        token
            .view
            .team_model_aliases
            .as_ref()
            .map(|payload| key_model_aliases(&payload.0)),
        Some(key_model_aliases(&fields.aliases.0)),
        fields.team_id.clone(),
    )
}

// can_key_call_resolved_model rebuilds the team from the key when the lookup fails
pub fn fallback_team(
    token: &UserAPIKeyAuth,
) -> Result<LiteLLMTeamTableCachedObj, EntityModelAccessError> {
    let view = &token.view;
    let Some(team_id) = view.token.team_id.clone() else {
        return Err(EntityModelAccessError::TeamRow(Box::new(
            TeamRowError::MissingTeamId,
        )));
    };
    let models: Vec<JsonData> = text_models(&view.team_models)
        .into_iter()
        .map(JsonData::String)
        .collect();
    let mut row = BTreeMap::new();
    row.insert("team_id".to_owned(), JsonData::String(team_id));
    row.insert("models".to_owned(), JsonData::Array(models));
    row.insert("blocked".to_owned(), JsonData::Bool(view.team_blocked));
    if let Some(team_alias) = &view.team_alias {
        row.insert(
            "team_alias".to_owned(),
            JsonData::String(team_alias.clone()),
        );
    }
    if let Some(metadata) = &view.team_metadata {
        row.insert("metadata".to_owned(), JsonData::Object(metadata.0.clone()));
    }
    if let Some(permission_id) = &view.team_object_permission_id {
        row.insert(
            "object_permission_id".to_owned(),
            JsonData::String(permission_id.clone()),
        );
    }
    if let Some(permission) = &token.extra.team_object_permission {
        let wire = serde_json::to_value(permission).map_err(|source| {
            EntityModelAccessError::TeamRow(Box::new(TeamRowError::Json(source)))
        })?;
        let data = serde_json::from_value::<JsonData>(wire).map_err(|source| {
            EntityModelAccessError::TeamRow(Box::new(TeamRowError::Json(source)))
        })?;
        row.insert("object_permission".to_owned(), data);
    }
    let team = LiteLLMTeamTable::try_from(JsonData::Object(row))
        .map_err(|error| EntityModelAccessError::TeamRow(Box::new(error)))?;
    Ok(LiteLLMTeamTableCachedObj {
        team,
        last_refreshed_at: None,
    })
}

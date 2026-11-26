use std::collections::BTreeMap;
use std::str::from_utf8;

use async_trait::async_trait;
use betterlite_config::loader::parse_view;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_logging::private_json::safe_json::safe_json_loads;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use crate::auth_team_lookup::error::TeamLookupError;
use crate::auth_team_lookup::lookup::{TeamLookup, TeamRequest, TeamSource};
use crate::auth_team_lookup::team::get_team_object;

pub const MAX_ROUTER_SETTINGS_BYTES: usize = 0x0010_0000;
pub const MAX_ROUTER_SETTINGS_DEPTH: usize = 64;

#[must_use]
pub fn decode_router_settings(bytes: &[u8]) -> Option<JsonObject> {
    if bytes.len() > MAX_ROUTER_SETTINGS_BYTES {
        return None;
    }
    let Ok(text) = from_utf8(bytes) else {
        return json_router_settings(bytes);
    };
    match parse_view::<Json>(text) {
        Ok((parsed, _ignored)) => settings_object(parsed),
        Err(_yaml_error) => json_router_settings(bytes),
    }
}

fn json_router_settings(bytes: &[u8]) -> Option<JsonObject> {
    let value = safe_json_loads(bytes).ok()?;
    let parsed: Json = serde_json::from_value(value).ok()?;
    settings_object(parsed)
}

fn settings_object(parsed: Json) -> Option<JsonObject> {
    if json_depth(&parsed) > MAX_ROUTER_SETTINGS_DEPTH {
        return None;
    }
    match parsed {
        Json::Object(entries) if !entries.is_empty() => Some(entries),
        Json::Null
        | Json::Bool(_)
        | Json::Number(_)
        | Json::Text(_)
        | Json::List(_)
        | Json::Object(_) => None,
    }
}

fn json_depth(value: &Json) -> usize {
    match value {
        Json::Object(entries) => entries
            .values()
            .map(json_depth)
            .max()
            .unwrap_or(0)
            .saturating_add(1),
        Json::List(items) => items
            .iter()
            .map(json_depth)
            .max()
            .unwrap_or(0)
            .saturating_add(1),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => 0,
    }
}

#[must_use]
pub fn parse_router_settings_value(value: Option<&Json>) -> Option<JsonObject> {
    match value {
        Some(Json::Object(entries)) if entries.is_empty() => None,
        Some(Json::Object(entries)) => Some(entries.clone()),
        Some(Json::Text(text)) => decode_router_settings(text.as_bytes()),
        None | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_)) => None,
    }
}

#[async_trait]
pub trait TeamRouterSettings: Send + Sync {
    async fn router_settings(
        &self,
        team_id: &str,
    ) -> Result<Option<BTreeMap<String, JsonData>>, TeamLookupError>;
}

pub async fn hierarchical_router_settings(
    key_settings: Option<&Json>,
    team_id: Option<&str>,
    teams: &dyn TeamRouterSettings,
) -> Option<JsonObject> {
    if let Some(settings) = parse_router_settings_value(key_settings) {
        return Some(settings);
    }
    let requested_team = team_id?;
    match teams.router_settings(requested_team).await {
        Ok(Some(columns)) => parse_router_settings_value(Some(&json_object(&columns))),
        Ok(None) => None,
        Err(error) => {
            tracing::debug!(team_id = requested_team, error = %error, "the team router settings lookup failed");
            None
        }
    }
}

#[async_trait]
impl TeamRouterSettings for TeamLookup {
    async fn router_settings(
        &self,
        team_id: &str,
    ) -> Result<Option<BTreeMap<String, JsonData>>, TeamLookupError> {
        let cached = get_team_object(
            self,
            TeamRequest {
                team_id,
                source: TeamSource::CacheThenDatabase,
                team_id_upsert: false,
            },
        )
        .await?;
        Ok(cached.team.fields.base.router_settings)
    }
}

fn json_object(columns: &BTreeMap<String, JsonData>) -> Json {
    Json::Object(
        columns
            .iter()
            .map(|(name, entry)| (name.clone(), json_value(entry)))
            .collect(),
    )
}

fn json_value(value: &JsonData) -> Json {
    match value {
        JsonData::Null => Json::Null,
        JsonData::Bool(flag) => Json::Bool(*flag),
        JsonData::Number(number) => Json::Number(number.clone()),
        JsonData::String(text) => Json::Text(text.clone()),
        JsonData::Array(items) => Json::List(items.iter().map(json_value).collect()),
        JsonData::Object(entries) => Json::Object(
            entries
                .iter()
                .map(|(name, entry)| (name.clone(), json_value(entry)))
                .collect(),
        ),
    }
}

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

pub const MAX_ROUTER_SETTINGS_BYTES: usize = 1_048_576;
pub const MAX_ROUTER_SETTINGS_DEPTH: usize = 64;

// decode_router_settings: YAML first, JSON only after a YAML parse error, and never a non-object.
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

// The same container count safe_json_loads enforces, so the YAML and JSON paths share one bound.
fn json_depth(value: &Json) -> usize {
    let mut pending = vec![(value, 0_usize)];
    let mut deepest = 0;
    while let Some((node, depth)) = pending.pop() {
        let next = depth.saturating_add(1);
        match node {
            Json::Object(entries) => {
                pending.extend(entries.values().map(|entry| (entry, next)));
                deepest = deepest.max(next);
            }
            Json::List(items) => {
                pending.extend(items.iter().map(|item| (item, next)));
                deepest = deepest.max(next);
            }
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => {}
        }
    }
    deepest
}

// _parse_router_settings_value: a dictionary is used as given, a string goes through the decoder.
#[must_use]
pub fn parse_router_settings_value(value: Option<&Json>) -> Option<JsonObject> {
    match value {
        Some(Json::Object(entries)) if entries.is_empty() => None,
        Some(Json::Object(entries)) => Some(entries.clone()),
        Some(Json::Text(text)) => decode_router_settings(text.as_bytes()),
        None | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_)) => None,
    }
}

// The cached team read _get_hierarchical_router_settings needs, injected so the caller owns B737.
#[async_trait]
pub trait TeamRouterSettings: Send + Sync {
    async fn router_settings(
        &self,
        team_id: &str,
    ) -> Result<Option<BTreeMap<String, JsonData>>, TeamLookupError>;
}

// _get_hierarchical_router_settings: key settings win, the cached team is the fallback, and nothing merges.
pub async fn hierarchical_router_settings(
    key_settings: Option<&Json>,
    team_id: Option<&str>,
    teams: &dyn TeamRouterSettings,
) -> Option<JsonObject> {
    if let Some(settings) = parse_router_settings_value(key_settings) {
        return Some(settings);
    }
    let team_id = team_id?;
    match teams.router_settings(team_id).await {
        Ok(Some(columns)) => parse_router_settings_value(Some(&json_object(&columns))),
        Ok(None) => None,
        Err(error) => {
            tracing::debug!(team_id, error = %error, "the team router settings lookup failed");
            None
        }
    }
}

// The B737 lookup is the production source: the cached team row, read through the shared object lookup.
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
            .map(|(name, value)| (name.clone(), json_value(value)))
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
                .map(|(name, value)| (name.clone(), json_value(value)))
                .collect(),
        ),
    }
}

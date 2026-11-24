use std::collections::BTreeMap;

use axum::Json;
use axum::extract::State;
use betterlite_core::api::claude_plugin_policy::{ClaudePluginPolicyError, DB_NOT_CONNECTED};
use serde::Deserialize;

use super::ResourceHubsState;
use super::error::ResourceHubsError;
use super::plugin_table::{PluginRow, PluginTable};
use super::plugin_types::{ListPluginsResponse, PluginAuthor, PluginListItem};
use super::timestamps::python_utc_timestamp;

#[derive(Debug, Default, Deserialize)]
struct Manifest {
    #[serde(default, with = "serde_with::rust::double_option")]
    source: Option<Option<BTreeMap<String, String>>>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    author: Option<PluginAuthor>,
    #[serde(default)]
    homepage: Option<String>,
    #[serde(default)]
    keywords: Option<Vec<String>>,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    domain: Option<String>,
    #[serde(default)]
    namespace: Option<String>,
}

pub async fn skill_hub(
    State(state): State<ResourceHubsState>,
) -> Result<Json<ListPluginsResponse>, ResourceHubsError> {
    // B1140's require_database answers a missing database with this 500; the route also needs the table itself.
    let Some(table) = state.plugins.as_ref() else {
        return Err(ResourceHubsError::PluginPolicy(
            ClaudePluginPolicyError::DatabaseNotConnected {
                message: DB_NOT_CONNECTED,
            },
        ));
    };
    Ok(Json(plugin_response(table.as_ref()).await?))
}

#[must_use]
pub async fn plugin_response(
    table: &dyn PluginTable,
) -> Result<ListPluginsResponse, ResourceHubsError> {
    let rows = table
        .enabled_plugins()
        .await
        .map_err(ResourceHubsError::PluginTable)?;
    let mut plugins = Vec::new();
    for row in &rows {
        plugins.push(plugin_item(row)?);
    }
    let count = plugins.len();
    Ok(ListPluginsResponse { plugins, count })
}

fn plugin_item(row: &PluginRow) -> Result<PluginListItem, ResourceHubsError> {
    let Manifest {
        source,
        version,
        description,
        author,
        homepage,
        keywords,
        category,
        domain,
        namespace,
    } = manifest_of(row.manifest_json.as_deref())?;
    let resolved_source = match source {
        None => BTreeMap::new(),
        Some(None) => return Err(ResourceHubsError::ManifestSource),
        Some(Some(entries)) => entries,
    };
    Ok(PluginListItem {
        id: row.id.clone(),
        name: row.name.clone(),
        version,
        description,
        source: resolved_source,
        author,
        homepage,
        keywords,
        category,
        domain,
        namespace,
        enabled: row.enabled,
        created_at: row.created_at.map(python_utc_timestamp),
        updated_at: row.updated_at.map(python_utc_timestamp),
    })
}

fn manifest_of(raw: Option<&str>) -> Result<Manifest, ResourceHubsError> {
    let Some(text) = raw.filter(|text| !text.is_empty()) else {
        return Ok(Manifest::default());
    };
    serde_json::from_str(text).map_err(ResourceHubsError::Manifest)
}

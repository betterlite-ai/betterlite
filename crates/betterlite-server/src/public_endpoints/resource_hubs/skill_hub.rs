use std::collections::BTreeMap;

use axum::Json;
use axum::extract::State;
use betterlite_core::api::claude_plugin_policy::{ClaudePluginPolicyError, DB_NOT_CONNECTED};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::value::RawValue;

use super::ResourceHubsState;
use super::error::ResourceHubsError;
use super::plugin_table::{PluginRow, PluginTable};
use super::plugin_types::{ListPluginsResponse, PluginAuthor, PluginListItem};
use super::timestamps::utc_timestamp_text;

#[derive(Debug, Default)]
struct Manifest {
    source: SourceField,
    version: Option<String>,
    description: Option<String>,
    author: Option<PluginAuthor>,
    homepage: Option<String>,
    keywords: Option<Vec<String>>,
    category: Option<String>,
    domain: Option<String>,
    namespace: Option<String>,
}

// the manifest's source key: an absent key means {}, an explicit null is refused, and an object is the map
#[derive(Debug, Default, Deserialize)]
#[serde(untagged)]
enum SourceField {
    Null(()),
    Map(BTreeMap<String, String>),
    #[default]
    Absent,
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
        SourceField::Absent => BTreeMap::new(),
        SourceField::Null(()) => return Err(ResourceHubsError::ManifestSource),
        SourceField::Map(entries) => entries,
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
        created_at: row.created_at.map(utc_timestamp_text),
        updated_at: row.updated_at.map(utc_timestamp_text),
    })
}

// LiteLLM calls .get on the parsed manifest, so anything but a JSON object fails the request
fn manifest_of(raw: Option<&str>) -> Result<Manifest, ResourceHubsError> {
    let Some(text) = raw.filter(|text| !text.is_empty()) else {
        return Ok(Manifest::default());
    };
    let entries: BTreeMap<String, Box<RawValue>> =
        serde_json::from_str(text).map_err(ResourceHubsError::Manifest)?;
    let source = match entries.get("source") {
        Some(encoded) => {
            serde_json::from_str(encoded.get()).map_err(ResourceHubsError::Manifest)?
        }
        None => SourceField::default(),
    };
    Ok(Manifest {
        source,
        version: manifest_field(&entries, "version")?,
        description: manifest_field(&entries, "description")?,
        author: manifest_field(&entries, "author")?,
        homepage: manifest_field(&entries, "homepage")?,
        keywords: manifest_field(&entries, "keywords")?,
        category: manifest_field(&entries, "category")?,
        domain: manifest_field(&entries, "domain")?,
        namespace: manifest_field(&entries, "namespace")?,
    })
}

fn manifest_field<T>(
    entries: &BTreeMap<String, Box<RawValue>>,
    key: &str,
) -> Result<Option<T>, ResourceHubsError>
where
    T: DeserializeOwned,
{
    let Some(raw) = entries.get(key) else {
        return Ok(None);
    };
    serde_json::from_str::<Option<T>>(raw.get()).map_err(ResourceHubsError::Manifest)
}

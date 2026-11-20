use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use serde::Deserialize;

use super::error::CleanupError;

pub type AliasMap = JsonObject;

// One litellm_modeltable row with its team relation, as delete_team_model_alias reads it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct AliasRow {
    pub id: i64,
    pub team_id: Option<String>,
    pub aliases: Option<AliasMap>,
}

#[async_trait]
pub trait AliasRows: Send + Sync {
    async fn alias_rows(&self) -> Result<Vec<AliasRow>, CleanupError>;

    async fn save_aliases(&self, id: i64, aliases: &AliasMap) -> Result<(), CleanupError>;
}

// delete_team_model_alias: the first key mapping to the public name goes, and only a row with a team reports the pair.
pub async fn delete_team_model_alias(
    public_model_name: &str,
    rows: &dyn AliasRows,
) -> Result<Vec<(String, String)>, CleanupError> {
    let mut removed = Vec::new();
    let mut failure: Option<CleanupError> = None;
    for row in rows.alias_rows().await? {
        let Some(aliases) = row.aliases else {
            continue;
        };
        let Some(key) = first_key_for(&aliases, public_model_name) else {
            continue;
        };
        let mut updated = aliases;
        updated.shift_remove(&key);
        if let Some(team_id) = row.team_id {
            removed.push((team_id, key));
        }
        if let Err(error) = rows.save_aliases(row.id, &updated).await {
            failure.get_or_insert(error);
        }
    }
    match failure {
        Some(error) => Err(error),
        None => Ok(removed),
    }
}

fn first_key_for(aliases: &AliasMap, public_model_name: &str) -> Option<String> {
    for (key, value) in aliases {
        if matches!(value, Json::Text(name) if name == public_model_name) {
            return Some(key.clone());
        }
    }
    None
}

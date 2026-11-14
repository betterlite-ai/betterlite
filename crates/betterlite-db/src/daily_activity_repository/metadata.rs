use super::database::DailyActivityDatabase;
use super::rows::records::{DeletedVerificationTokenRow, VerificationTokenRow};
use super::{DailyActivityRepository, Result};
use crate::budget_permission_rows::JsonData;
use crate::daily_activity_contracts::{DailyActivityProxyReads, KeyMetadataRow, SpendLogsWindow};
use std::collections::{BTreeMap, BTreeSet};

impl<Database, Reads, Clock> DailyActivityRepository<Database, Reads, Clock>
where
    Database: DailyActivityDatabase,
    Reads: DailyActivityProxyReads,
{
    pub async fn key_metadata(
        &self,
        api_keys: &BTreeSet<String>,
        window: Option<SpendLogsWindow>,
    ) -> Result<BTreeMap<String, KeyMetadataRow>> {
        if api_keys.is_empty() {
            return Ok(BTreeMap::new());
        }
        let values: Vec<String> = api_keys.iter().cloned().collect();
        let active_rows = self.database.active_tokens(&values).await?;
        let mut active = BTreeMap::new();
        for row in &active_rows {
            active.insert(row.token.clone(), metadata_row(row, true));
        }
        let mut missing = Vec::new();
        for key in &values {
            if !active.contains_key(key) {
                missing.push(key.clone());
            }
        }
        let deleted_rows = match self.database.deleted_tokens(&missing).await {
            Ok(rows) => rows,
            Err(error) => {
                tracing::warn!(
                    "Could not read deleted verification token metadata: {}",
                    error
                );
                Vec::new()
            }
        };
        let mut newest: BTreeMap<String, DeletedVerificationTokenRow> = BTreeMap::new();
        for row in deleted_rows {
            match newest.get(&row.row.token) {
                Some(existing) if existing.deleted_at >= row.deleted_at => {}
                _ => {
                    newest.insert(row.row.token.clone(), row);
                }
            }
        }
        let mut resolved = BTreeMap::new();
        for key in &missing {
            if let Some(row) = newest.get(key) {
                resolved.insert(key.clone(), metadata_row(&row.row, false));
            }
        }
        resolved.extend(active);
        Ok(self
            .proxy_reads
            .recover_key_metadata(resolved, api_keys, window)
            .await)
    }
}

#[must_use]
pub fn metadata_tags(value: Option<&JsonData>) -> Vec<String> {
    let Some(JsonData::Array(items)) = value else {
        return Vec::new();
    };
    let mut tags = Vec::with_capacity(items.len());
    for item in items {
        let JsonData::String(tag) = item else {
            return Vec::new();
        };
        tags.push(tag.clone());
    }
    tags
}

#[must_use]
pub fn metadata_row(row: &VerificationTokenRow, key_exists: bool) -> KeyMetadataRow {
    KeyMetadataRow {
        api_key: row.token.clone(),
        key_alias: row.key_alias.clone(),
        team_id: row.team_id.clone(),
        user_id: row.user_id.clone(),
        user_email: None,
        key_exists,
        tags: metadata_tags(
            row.metadata
                .as_ref()
                .and_then(|metadata| metadata.get("tags")),
        ),
    }
}

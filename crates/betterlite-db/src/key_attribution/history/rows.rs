use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use tokio_postgres::Row;

use crate::key_attribution::metadata::{KeyMetadata, unanimous};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpendLogDigestRow {
    pub digest: String,
    #[serde(default)]
    pub first_alias: Option<String>,
    #[serde(default)]
    pub last_alias: Option<String>,
    #[serde(default)]
    pub first_team: Option<String>,
    #[serde(default)]
    pub last_team: Option<String>,
    #[serde(default)]
    pub first_owner: Option<String>,
    #[serde(default)]
    pub last_owner: Option<String>,
}

impl SpendLogDigestRow {
    #[must_use]
    pub fn metadata(&self) -> KeyMetadata {
        KeyMetadata {
            key_alias: unanimous(self.first_alias.as_deref(), self.last_alias.as_deref()),
            team_id: unanimous(self.first_team.as_deref(), self.last_team.as_deref()),
            user_id: unanimous(self.first_owner.as_deref(), self.last_owner.as_deref()),
            user_email: None,
            key_exists: None,
        }
    }
}

pub fn spend_log_values(row: &Row) -> Result<SpendLogDigestRow, tokio_postgres::Error> {
    Ok(SpendLogDigestRow {
        digest: row.try_get("digest")?,
        first_alias: row.try_get("first_alias")?,
        last_alias: row.try_get("last_alias")?,
        first_team: row.try_get("first_team")?,
        last_team: row.try_get("last_team")?,
        first_owner: row.try_get("first_owner")?,
        last_owner: row.try_get("last_owner")?,
    })
}

#[must_use]
pub fn edge_metadata(
    rows: &[SpendLogDigestRow],
    requested: &BTreeSet<String>,
) -> BTreeMap<String, KeyMetadata> {
    rows.iter()
        .filter(|row| requested.contains(&row.digest))
        .map(|row| (row.digest.clone(), row.metadata()))
        .filter(|(_, metadata)| has_nonempty_value(metadata))
        .collect()
}

fn has_nonempty_value(metadata: &KeyMetadata) -> bool {
    [&metadata.key_alias, &metadata.team_id, &metadata.user_id]
        .into_iter()
        .any(|value| value.as_deref().is_some_and(|text| !text.is_empty()))
}

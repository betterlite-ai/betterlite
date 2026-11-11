use serde::{Deserialize, Serialize};

use crate::claude_settings::SettingsDocument;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BackupRecord {
    pub existed: bool,
    // serializing an absent value is fine, but serde would otherwise accept a backup that never wrote the key
    #[serde(deserialize_with = "Option::deserialize")]
    pub content: Option<SettingsDocument>,
}

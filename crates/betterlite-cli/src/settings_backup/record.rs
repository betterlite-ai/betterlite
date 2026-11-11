use serde::{Deserialize, Serialize};

use crate::claude_settings::SettingsDocument;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BackupRecord {
    pub existed: bool,
    pub content: Option<SettingsDocument>,
}

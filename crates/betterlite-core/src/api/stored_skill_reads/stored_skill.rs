use std::collections::BTreeMap;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::managed_resource_rows::skills::SkillTable;
use betterlite_db::managed_resource_rows::wire::RowTimestamp;

use super::error::StoredSkillReadError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredSkill {
    pub skill_id: String,
    pub display_title: Option<String>,
    pub description: Option<String>,
    pub instructions: Option<String>,
    pub source: String,
    pub latest_version: Option<String>,
    pub file_content: Option<Vec<u8>>,
    pub file_name: Option<String>,
    pub file_type: Option<String>,
    pub metadata: Option<BTreeMap<String, JsonData>>,
    pub created_at: Option<RowTimestamp>,
    pub created_by: Option<String>,
    pub updated_at: Option<RowTimestamp>,
    pub updated_by: Option<String>,
}

impl StoredSkill {
    pub fn from_row(row: SkillTable) -> Result<Self, StoredSkillReadError> {
        let file_content = row
            .file_content
            .as_deref()
            .map(decode_file_content)
            .transpose()?;
        Ok(Self {
            skill_id: row.skill_id,
            display_title: row.display_title,
            description: row.description,
            instructions: row.instructions,
            source: row.source,
            latest_version: row.latest_version,
            file_content,
            file_name: row.file_name,
            file_type: row.file_type,
            metadata: row.metadata,
            created_at: row.created_at,
            created_by: row.created_by,
            updated_at: row.updated_at,
            updated_by: row.updated_by,
        })
    }
}

// PostgreSQL's encode(..., 'base64') wraps at 76 columns, and Python's b64decode ignores that whitespace
fn decode_file_content(encoded: &str) -> Result<Vec<u8>, StoredSkillReadError> {
    let stripped: String = encoded
        .chars()
        .filter(|character| !character.is_ascii_whitespace())
        .collect();
    STANDARD
        .decode(stripped.as_bytes())
        .map_err(StoredSkillReadError::FileContent)
}

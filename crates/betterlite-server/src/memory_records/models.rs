use betterlite_db::budget_permission_rows::{JsonData, Timestamp};
use serde::{Deserialize, Serialize};

// types/memory_management.py LiteLLM_MemoryRow: the stored row projected with all ten fields, nulls included
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryRow {
    pub memory_id: String,
    pub key: String,
    pub value: String,
    #[serde(default)]
    pub metadata: Option<JsonData>,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub team_id: Option<String>,
    #[serde(default)]
    pub created_at: Option<Timestamp>,
    #[serde(default)]
    pub created_by: Option<String>,
    #[serde(default)]
    pub updated_at: Option<Timestamp>,
    #[serde(default)]
    pub updated_by: Option<String>,
}

// types/memory_management.py MemoryCreateRequest: key and value are required, the scope is overridable
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateRequest {
    pub key: String,
    pub value: String,
    #[serde(default)]
    pub metadata: Option<JsonData>,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub team_id: Option<String>,
}

// types/memory_management.py MemoryUpdateRequest: the outer Option is field presence, so an absent
// field and an explicit null stay apart; only presence puts the metadata column in the update
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateRequest {
    #[serde(default, with = "serde_with::rust::double_option")]
    pub value: Option<Option<String>>,
    #[serde(default, with = "serde_with::rust::double_option")]
    pub metadata: Option<Option<JsonData>>,
    #[serde(default, with = "serde_with::rust::double_option")]
    pub user_id: Option<Option<String>>,
    #[serde(default, with = "serde_with::rust::double_option")]
    pub team_id: Option<Option<String>>,
}

impl UpdateRequest {
    // body.value is not None: an explicit null updates nothing, like an absent field
    #[must_use]
    pub fn value(&self) -> Option<&str> {
        match &self.value {
            Some(Some(value)) => Some(value),
            Some(None) | None => None,
        }
    }

    // body.model_fields_set, sorted so the answer does not depend on the payload's order
    #[must_use]
    pub fn fields_sent(&self) -> Vec<&'static str> {
        let mut sent = Vec::new();
        if self.value.is_some() {
            sent.push("value");
        }
        if self.metadata.is_some() {
            sent.push("metadata");
        }
        if self.user_id.is_some() {
            sent.push("user_id");
        }
        if self.team_id.is_some() {
            sent.push("team_id");
        }
        sent.sort_unstable();
        sent
    }
}

// types/memory_management.py MemoryListResponse
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListResponse {
    pub memories: Vec<MemoryRow>,
    pub total: i64,
}

// types/memory_management.py MemoryDeleteResponse
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteResponse {
    pub key: String,
    pub deleted: bool,
}

impl CreateRequest {
    // memory_endpoints.py _serialize_metadata_for_prisma at the create call sites: a value is
    // stored as JSON text, and a null leaves the field out so the column stays SQL NULL
    pub fn metadata_column(&self) -> Result<Option<String>, serde_json::Error> {
        let Some(metadata) = self.metadata.as_ref() else {
            return Ok(None);
        };
        Ok(Some(serde_json::to_string(metadata)?))
    }
}

impl UpdateRequest {
    // memory_endpoints.py upsert_memory: an absent metadata key leaves the column alone, and a
    // present one writes JSON text where an explicit null becomes the literal null
    pub fn metadata_column(&self) -> Result<Option<String>, serde_json::Error> {
        let Some(metadata) = self.metadata.as_ref() else {
            return Ok(None);
        };
        let value = metadata.as_ref().unwrap_or(&JsonData::Null);
        Ok(Some(serde_json::to_string(value)?))
    }
}

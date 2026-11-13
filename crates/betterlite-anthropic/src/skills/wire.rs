use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

const SKILL_TYPE: &str = "skill";
const DELETED_SKILL_TYPE: &str = "skill_deleted";

// TypedDict(total=False) for create_skill; extra_body overlays keys the TypedDict does not declare
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct CreateSkillRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<Json>>,
    #[serde(flatten)]
    pub extra: IndexMap<String, Json>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct ListSkillsParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(from = "RawSkill")]
pub struct Skill {
    pub id: String,
    pub created_at: String,
    pub display_title: Option<String>,
    pub latest_version: Option<String>,
    pub source: String,
    pub r#type: String,
    pub updated_at: String,
    pub description: Option<String>,
    pub search_score: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(from = "RawSkillList")]
pub struct ListSkillsResponse {
    pub data: Vec<Skill>,
    pub next_page: Option<String>,
    pub has_more: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(from = "RawDeletedSkill")]
pub struct DeleteSkillResponse {
    pub id: String,
    pub r#type: String,
}

#[derive(Debug, Deserialize)]
struct RawSkill {
    id: String,
    created_at: String,
    #[serde(default)]
    display_title: Option<String>,
    #[serde(default)]
    latest_version: Option<String>,
    source: String,
    #[serde(default = "default_skill_type")]
    r#type: String,
    updated_at: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    search_score: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct RawSkillList {
    data: Vec<RawSkill>,
    #[serde(default)]
    next_page: Option<String>,
    #[serde(default = "default_has_more")]
    has_more: bool,
}

#[derive(Debug, Deserialize)]
struct RawDeletedSkill {
    id: String,
    #[serde(default = "default_deleted_skill_type")]
    r#type: String,
}

fn default_skill_type() -> String {
    SKILL_TYPE.to_owned()
}

fn default_deleted_skill_type() -> String {
    DELETED_SKILL_TYPE.to_owned()
}

const fn default_has_more() -> bool {
    false
}

impl From<RawSkill> for Skill {
    fn from(raw: RawSkill) -> Self {
        Self {
            id: raw.id,
            created_at: raw.created_at,
            display_title: raw.display_title,
            latest_version: raw.latest_version,
            source: raw.source,
            r#type: raw.r#type,
            updated_at: raw.updated_at,
            description: raw.description,
            search_score: raw.search_score,
        }
    }
}

impl From<RawSkillList> for ListSkillsResponse {
    fn from(raw: RawSkillList) -> Self {
        Self {
            data: raw.data.into_iter().map(Skill::from).collect(),
            next_page: raw.next_page,
            has_more: raw.has_more,
        }
    }
}

impl From<RawDeletedSkill> for DeleteSkillResponse {
    fn from(raw: RawDeletedSkill) -> Self {
        Self {
            id: raw.id,
            r#type: raw.r#type,
        }
    }
}

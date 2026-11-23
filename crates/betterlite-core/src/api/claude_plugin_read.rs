pub mod error;

mod projection;

use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::api::claude_plugin_policy::source::PluginSource;
use crate::api::claude_plugin_policy::{
    PluginCaller, PluginRecord, PluginWhere, SkillVisibility, plugin_not_found_error,
    skill_visibility,
};

use self::error::{ClaudePluginReadFailure, PluginManifestError, plugin_not_granted_error};
use self::projection::{detail, list_item};

const AUTHOR_FIELD: &str = "author";
const HOMEPAGE_FIELD: &str = "homepage";
const KEYWORDS_FIELD: &str = "keywords";
const CATEGORY_FIELD: &str = "category";
const DOMAIN_FIELD: &str = "domain";
const NAMESPACE_FIELD: &str = "namespace";
const SOURCE_FIELD: &str = "source";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClaudePluginReadOperation {
    List { enabled_only: bool },
    Get { plugin_name: String },
}

#[derive(Clone, Debug)]
pub struct ClaudePluginReadInput {
    pub operation: ClaudePluginReadOperation,
    pub caller: Option<PluginCaller>,
}

// The manifest keys both read handlers use, as the stored JSON has them; a missing key and an
// explicit null stay apart because list_plugins shows the first as {} and refuses the second.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PluginManifestFields {
    pub source: Option<Json>,
    pub author: Option<Json>,
    pub homepage: Option<Json>,
    pub keywords: Option<Json>,
    pub category: Option<Json>,
    pub domain: Option<Json>,
    pub namespace: Option<Json>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginRow {
    pub id: String,
    pub name: String,
    pub version: Option<String>,
    pub description: Option<String>,
    pub enabled: bool,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub created_by: Option<String>,
    pub manifest: PluginManifestFields,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PluginListAuthor {
    pub name: String,
    pub email: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PluginListItem {
    pub id: String,
    pub name: String,
    pub version: Option<String>,
    pub description: Option<String>,
    pub source: PluginSource,
    pub author: Option<PluginListAuthor>,
    pub homepage: Option<String>,
    pub keywords: Option<Vec<String>>,
    pub category: Option<String>,
    pub domain: Option<String>,
    pub namespace: Option<String>,
    pub enabled: bool,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PluginList {
    pub plugins: Vec<PluginListItem>,
    pub count: usize,
}

// get_plugin builds its dict by hand, so source and the metadata keys stay as the manifest holds them
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PluginDetail {
    pub id: String,
    pub name: String,
    pub version: Option<String>,
    pub description: Option<String>,
    pub source: Json,
    pub author: Json,
    pub homepage: Json,
    pub keywords: Json,
    pub category: Json,
    pub enabled: bool,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub created_by: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ClaudePluginReadResult {
    List(PluginList),
    Detail(Box<PluginDetail>),
}

impl PluginManifestFields {
    pub fn from_manifest(manifest: Json) -> Result<Self, PluginManifestError> {
        let Json::Object(mut fields) = manifest else {
            return Err(PluginManifestError::NotAnObject);
        };
        Ok(Self {
            source: fields.shift_remove(SOURCE_FIELD),
            author: fields.shift_remove(AUTHOR_FIELD),
            homepage: fields.shift_remove(HOMEPAGE_FIELD),
            keywords: fields.shift_remove(KEYWORDS_FIELD),
            category: fields.shift_remove(CATEGORY_FIELD),
            domain: fields.shift_remove(DOMAIN_FIELD),
            namespace: fields.shift_remove(NAMESPACE_FIELD),
        })
    }
}

impl ClaudePluginReadInput {
    #[must_use]
    pub fn visibility(&self) -> SkillVisibility {
        skill_visibility(self.caller.as_ref())
    }

    // find_many's where clause: enabled_only replaces the visibility filter, and both handlers
    // still drop every row visibility.allows refuses after the query
    #[must_use]
    pub fn list_filter(&self) -> PluginWhere {
        match &self.operation {
            ClaudePluginReadOperation::List { enabled_only: true } => PluginWhere {
                enabled: Some(true),
                any: None,
            },
            ClaudePluginReadOperation::List {
                enabled_only: false,
            } => self.visibility().where_clause(),
            ClaudePluginReadOperation::Get { .. } => PluginWhere {
                enabled: None,
                any: None,
            },
        }
    }

    pub fn run(
        &self,
        rows: Vec<PluginRow>,
    ) -> Result<ClaudePluginReadResult, ClaudePluginReadFailure> {
        let visibility = self.visibility();
        match &self.operation {
            ClaudePluginReadOperation::List { .. } => {
                let mut plugins = Vec::with_capacity(rows.len());
                for row in rows {
                    if !visibility.allows(&record(&row)) {
                        continue;
                    }
                    plugins.push(list_item(row)?);
                }
                plugins.sort_by(|left, right| {
                    right
                        .created_at
                        .as_deref()
                        .unwrap_or_default()
                        .cmp(left.created_at.as_deref().unwrap_or_default())
                });
                let count = plugins.len();
                Ok(ClaudePluginReadResult::List(PluginList { plugins, count }))
            }
            ClaudePluginReadOperation::Get { plugin_name } => {
                let row = rows
                    .into_iter()
                    .next()
                    .ok_or_else(|| plugin_not_found_error(plugin_name))?;
                if !visibility.allows(&record(&row)) {
                    return Err(ClaudePluginReadFailure::from(plugin_not_granted_error(
                        plugin_name,
                    )));
                }
                Ok(ClaudePluginReadResult::Detail(Box::new(detail(row))))
            }
        }
    }
}

fn record(row: &PluginRow) -> PluginRecord {
    PluginRecord {
        name: row.name.clone(),
        enabled: row.enabled,
    }
}

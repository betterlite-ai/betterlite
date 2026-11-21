use serde::Deserialize;

use super::ModelAllowlistError;

// The closed five-table set: the kind literal and column names never come from input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum AllowlistKind {
    #[serde(rename = "team")]
    Team,
    #[serde(rename = "key")]
    Key,
    #[serde(rename = "org")]
    Organization,
    #[serde(rename = "project")]
    Project,
    #[serde(rename = "user")]
    User,
}

impl AllowlistKind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Team => "team",
            Self::Key => "key",
            Self::Organization => "org",
            Self::Project => "project",
            Self::User => "user",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        match name {
            "team" => Some(Self::Team),
            "key" => Some(Self::Key),
            "org" => Some(Self::Organization),
            "project" => Some(Self::Project),
            "user" => Some(Self::User),
            _ => None,
        }
    }
}

// One returned row of the five-table statement, decoded before any invalidation runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TouchedRow {
    pub kind: AllowlistKind,
    pub object_id: String,
    pub team_alias: Option<String>,
}

impl TouchedRow {
    pub(crate) fn from_values(
        kind: &str,
        object_id: String,
        team_alias: Option<String>,
    ) -> Result<Self, ModelAllowlistError> {
        let parsed =
            AllowlistKind::from_name(kind).ok_or_else(|| ModelAllowlistError::UnknownKind {
                kind: kind.to_owned(),
            })?;
        Ok(Self {
            kind: parsed,
            object_id,
            team_alias,
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct AllowlistTable {
    pub(crate) kind: AllowlistKind,
    pub(crate) table: &'static str,
    pub(crate) id_column: &'static str,
    pub(crate) alias_column: Option<&'static str>,
}

pub(crate) const ALLOWLIST_TABLES: [AllowlistTable; 5] = [
    AllowlistTable {
        kind: AllowlistKind::Team,
        table: "LiteLLM_TeamTable",
        id_column: "team_id",
        alias_column: Some("team_alias"),
    },
    AllowlistTable {
        kind: AllowlistKind::Key,
        table: "LiteLLM_VerificationToken",
        id_column: "token",
        alias_column: None,
    },
    AllowlistTable {
        kind: AllowlistKind::Organization,
        table: "LiteLLM_OrganizationTable",
        id_column: "organization_id",
        alias_column: None,
    },
    AllowlistTable {
        kind: AllowlistKind::Project,
        table: "LiteLLM_ProjectTable",
        id_column: "project_id",
        alias_column: None,
    },
    AllowlistTable {
        kind: AllowlistKind::User,
        table: "LiteLLM_UserTable",
        id_column: "user_id",
        alias_column: None,
    },
];

impl AllowlistTable {
    fn update_cte(&self, set_clause: &str, where_clause: &str) -> String {
        let kind = self.kind.as_str();
        let table = self.table;
        let id_column = self.id_column;
        let alias = self
            .alias_column
            .map_or_else(|| "NULL::text".to_owned(), |column| format!("\"{column}\""));
        format!(
            "{kind}_rows AS (UPDATE \"{table}\" SET \"models\" = {set_clause} WHERE {where_clause} RETURNING '{kind}' AS kind, \"{id_column}\" AS object_id, {alias} AS team_alias)"
        )
    }
}

// One statement touching every allowlist table, so the rewrite lands everywhere or nowhere.
pub(crate) fn rewrite_sql(set_clause: &str, where_clause: &str) -> String {
    let mut ctes: Vec<String> = Vec::with_capacity(ALLOWLIST_TABLES.len());
    let mut rows: Vec<String> = Vec::with_capacity(ALLOWLIST_TABLES.len());
    for table in &ALLOWLIST_TABLES {
        let kind = table.kind.as_str();
        ctes.push(table.update_cte(set_clause, where_clause));
        rows.push(format!(
            "SELECT kind, object_id, team_alias FROM {kind}_rows"
        ));
    }
    let joined_ctes = ctes.join(", ");
    let joined_rows = rows.join(" UNION ALL ");
    format!("WITH {joined_ctes} {joined_rows}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AllowlistRewrite {
    Append { old_name: String, new_name: String },
    Replace { old_name: String, new_name: String },
}

const REPLACE_SET_CLAUSE: &str = "array_replace(array_remove(\"models\", $2), $1, $2)";
const REPLACE_WHERE_CLAUSE: &str = "$1 = ANY(\"models\")";
const APPEND_SET_CLAUSE: &str = "array_append(\"models\", $2)";
const APPEND_WHERE_CLAUSE: &str = "$1 = ANY(\"models\") AND NOT ($2 = ANY(\"models\"))";

impl AllowlistRewrite {
    #[must_use]
    pub fn sql(&self) -> String {
        let (set_clause, where_clause) = match self {
            Self::Append { .. } => (APPEND_SET_CLAUSE, APPEND_WHERE_CLAUSE),
            Self::Replace { .. } => (REPLACE_SET_CLAUSE, REPLACE_WHERE_CLAUSE),
        };
        rewrite_sql(set_clause, where_clause)
    }
}

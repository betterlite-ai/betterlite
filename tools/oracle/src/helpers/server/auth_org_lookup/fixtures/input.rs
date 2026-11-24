use betterlite_auth::auth_prefetch::rows::CachedProject;
use betterlite_db::resource_rows::organization::OrganizationTable;
use betterlite_db::resource_rows::project::ProjectTable;
use serde::Deserialize;

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Function {
    OrgObject,
    OrgAlias,
    OrgForRequest,
    ProjectObject,
}

// `allow_requests_on_db_unavailable` as a case can spell it: a boolean or a string
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum Allow {
    Boolean(bool),
    Text(String),
}

#[derive(Debug, Deserialize)]
pub(crate) struct Step {
    #[serde(default)]
    pub(crate) now: f64,
    // the cache writes and database reads this step fails, by name: outage, fault or `write <key>`
    #[serde(default)]
    pub(crate) fail: Vec<String>,
    #[serde(default)]
    pub(crate) allow: Option<Allow>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) function: Function,
    pub(crate) database: bool,
    pub(crate) target: String,
    #[serde(default)]
    pub(crate) include_budget: bool,
    pub(crate) in_memory_ttl: f64,
    #[serde(default)]
    pub(crate) cached_org: Option<OrganizationTable>,
    #[serde(default)]
    pub(crate) cached_last_known: Option<OrganizationTable>,
    #[serde(default)]
    pub(crate) cached_project: Option<CachedProject>,
    #[serde(default)]
    pub(crate) org_row: Option<OrganizationTable>,
    #[serde(default)]
    pub(crate) alias_rows: Vec<OrganizationTable>,
    #[serde(default)]
    pub(crate) project_row: Option<ProjectTable>,
    pub(crate) steps: Vec<Step>,
}

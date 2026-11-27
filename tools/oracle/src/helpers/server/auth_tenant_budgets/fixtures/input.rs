use std::collections::BTreeMap;

use betterlite_auth::auth_prefetch::rows::CachedProject;
use betterlite_auth::auth_tag_budgets::groups::ModelAccessGroupBudget;
use betterlite_core::api::request_destination::RequestData;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::managed_resource_rows::tag::TagTable;
use betterlite_db::resource_rows::organization::OrganizationTable;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) calls: Vec<Call>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Call {
    pub(crate) function: Function,
    pub(crate) database: bool,
    pub(crate) spends: Vec<JsonData>,
    #[serde(default)]
    pub(crate) project: Option<CachedProject>,
    #[serde(default)]
    pub(crate) token: Option<BTreeMap<String, JsonData>>,
    #[serde(default)]
    pub(crate) team: Option<LiteLLMTeamTable>,
    #[serde(default)]
    pub(crate) organization: Option<OrganizationTable>,
    #[serde(default)]
    pub(crate) org_raises: bool,
    #[serde(default)]
    pub(crate) request_body: Option<RequestData>,
    #[serde(default)]
    pub(crate) tag_rows: Vec<TagTable>,
    #[serde(default)]
    pub(crate) matched_model_access_groups: Vec<String>,
    #[serde(default)]
    pub(crate) group_rows: Vec<ModelAccessGroupBudget>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Function {
    ProjectHard,
    ProjectSoft,
    Organization,
    Tags,
    Groups,
}

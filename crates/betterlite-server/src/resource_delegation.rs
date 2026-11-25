pub mod admin_grant;
pub mod denial;
pub mod gates;
pub mod grandfather;
pub mod requested;
pub mod server_validation;
pub mod team_read;

mod tests;

use betterlite_auth::auth_model_patterns::access_groups::AccessGroupLookup;
use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_mcp::registry::servers::SharedRegistry;

#[derive(Clone, Debug)]
pub struct ObjectPermissionGrants {
    pub mcp_servers: Vec<String>,
    pub mcp_access_groups: Vec<String>,
    pub mcp_tool_permissions: JsonData,
    pub mcp_toolsets: Vec<String>,
    pub search_tools: Vec<String>,
}

pub struct GrantReads<'reads, Rows> {
    pub client: Option<PrismaClient<'reads>>,
    pub registry: &'reads SharedRegistry,
    pub rows: Option<&'reads Rows>,
    pub unified: Option<&'reads dyn AccessGroupLookup>,
}

// Python renders sorted(...) inside the denial detail, so the text keeps its quoted, bracketed list shape.
#[must_use]
pub(crate) fn quoted_list<Values>(values: Values) -> String
where
    Values: IntoIterator,
    Values::Item: AsRef<str>,
{
    let mut ordered: Vec<String> = values
        .into_iter()
        .map(|value| value.as_ref().to_owned())
        .collect();
    ordered.sort_unstable();
    let quoted: Vec<String> = ordered.iter().map(|value| format!("'{value}'")).collect();
    format!("[{}]", quoted.join(", "))
}

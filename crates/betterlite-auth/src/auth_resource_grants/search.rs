use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::LiteLLMObjectPermissionTable;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use betterlite_error::proxy_error_types::ProxyErrorTypes;
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};

use crate::resolved_key::auth::UserAPIKeyAuth;

// the object_type literals _can_object_call_search_tool formats into its denial
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchToolObject {
    Key,
    Team,
    Project,
}

// a permission source answers with a denial, or fails for a reason that is not a permission decision
#[derive(Debug, thiserror::Error)]
pub enum SearchToolFailure {
    #[error(transparent)]
    Denied(#[from] ProxyException),
    #[error("the search tool permission source failed: {message}")]
    Lookup { message: String },
}

// can_user_view_search_tool's two checks, so a source failure that is not a denial reaches the caller
pub trait SearchToolGrant {
    fn authorize(&self, name: &str) -> Result<(), SearchToolFailure>;
}

pub struct KeySearchGrant<'token> {
    pub token: &'token UserAPIKeyAuth,
}

pub struct TeamSearchGrant<'team> {
    pub team: Option<&'team LiteLLMTeamTable>,
}

impl SearchToolGrant for KeySearchGrant<'_> {
    fn authorize(&self, name: &str) -> Result<(), SearchToolFailure> {
        can_key_call_search_tool(name, self.token).map_err(SearchToolFailure::from)
    }
}

impl SearchToolGrant for TeamSearchGrant<'_> {
    fn authorize(&self, name: &str) -> Result<(), SearchToolFailure> {
        can_team_call_search_tool(name, self.team).map_err(SearchToolFailure::from)
    }
}

// an absent permission, a null search_tools and an empty one all leave the object unrestricted
#[must_use]
pub fn search_tool_names(permission: Option<&LiteLLMObjectPermissionTable>) -> Vec<String> {
    permission
        .and_then(|granted| granted.search_tools.0.as_ref())
        .cloned()
        .unwrap_or_default()
}

// LiteLLM grants nothing for a prefix or a wildcard: only an exact name in the allowlist passes
pub fn can_object_call_search_tool(
    search_tool_name: &str,
    allowed: &[String],
    object: SearchToolObject,
) -> Result<(), ProxyException> {
    if allowed.is_empty() || allowed.iter().any(|tool| tool == search_tool_name) {
        return Ok(());
    }
    let subject = match object {
        SearchToolObject::Key => "Key",
        SearchToolObject::Team => "Team",
        SearchToolObject::Project => "Project",
    };
    let message = format!(
        "{subject} not allowed to access search tool: {search_tool_name}. Allowed search tools: {}",
        allowed.join(", ")
    );
    Err(ProxyException::new(ProxyExceptionInit {
        message,
        error_type: ProxyErrorTypes::KeyModelAccessDenied.to_string(),
        param: Some("search_tool_name".to_owned()),
        code: ExceptionCode::Status(403),
        headers: BTreeMap::new(),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    }))
}

// the token's role is read by no check here, so a proxy admin gets no bypass from this helper
pub fn can_key_call_search_tool(
    search_tool_name: &str,
    valid_token: &UserAPIKeyAuth,
) -> Result<(), ProxyException> {
    let permission = valid_token.view.token.object_permission.as_ref();
    can_object_call_search_tool(
        search_tool_name,
        &search_tool_names(permission),
        SearchToolObject::Key,
    )
}

// a team denial still carries key_model_access_denied, as LiteLLM's shared projection does
pub fn can_team_call_search_tool(
    search_tool_name: &str,
    team_object: Option<&LiteLLMTeamTable>,
) -> Result<(), ProxyException> {
    let Some(team) = team_object else {
        return Ok(());
    };
    let permission = team.fields.object_permission.as_ref();
    can_object_call_search_tool(
        search_tool_name,
        &search_tool_names(permission),
        SearchToolObject::Team,
    )
}

// /search enforcement: the key first, then the team only when the key names one
pub fn authorize_search_tool(
    search_tool_name: &str,
    valid_token: &UserAPIKeyAuth,
    team_object: Option<&LiteLLMTeamTable>,
) -> Result<(), ProxyException> {
    can_key_call_search_tool(search_tool_name, valid_token)?;
    if valid_token.view.token.team_id.is_none() {
        return Ok(());
    }
    can_team_call_search_tool(search_tool_name, team_object)
}

// the boolean variant the search tool list scopes with: key first, then team, and False only for a denial
pub fn can_user_view_search_tool(
    search_tool_name: &str,
    key: &dyn SearchToolGrant,
    team: &dyn SearchToolGrant,
) -> Result<bool, SearchToolFailure> {
    match key.authorize(search_tool_name) {
        Ok(()) => {}
        Err(SearchToolFailure::Denied(_)) => return Ok(false),
        Err(failure) => return Err(failure),
    }
    match team.authorize(search_tool_name) {
        Ok(()) => Ok(true),
        Err(SearchToolFailure::Denied(_)) => Ok(false),
        Err(failure) => Err(failure),
    }
}

use std::collections::{BTreeMap, BTreeSet};

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use betterlite_error::proxy_error_types::ProxyErrorTypes;
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};
use betterlite_models::model_registration::JsonObject;

use crate::resolved_key::auth::UserAPIKeyAuth;

use super::extract::{
    ExtractionError, ToolCapableCallType, ToolNameExtractors, extract_request_tool_names,
};

const TOOL_DENIAL_PARAM: &str = "tools";
const TOOL_DENIAL_STATUS: i64 = 403;

#[derive(Debug, thiserror::Error)]
pub enum ToolCheckError {
    #[error(transparent)]
    Denied(Box<ProxyException>),
    #[error(transparent)]
    Extraction(#[from] ExtractionError),
}

// check_tools_allowlist: no token, no tool-capable route or no effective allowlist means nothing to enforce
pub fn check_tools_allowlist(
    request_body: &JsonObject,
    token: Option<&UserAPIKeyAuth>,
    _team: Option<&LiteLLMTeamTable>,
    route: &str,
    extractors: &ToolNameExtractors,
) -> Result<(), ToolCheckError> {
    let Some(token) = token else {
        return Ok(());
    };
    if !has_tool_capable_call_type(route, extractors) {
        return Ok(());
    }
    let tool_names = extract_request_tool_names(route, request_body, extractors)?;
    if tool_names.is_empty() {
        return Ok(());
    }
    let Some(effective) = effective_allowed_tools(token) else {
        return Ok(());
    };
    let allowed: BTreeSet<String> = effective.iter().map(allowed_string).collect();
    let disallowed: Vec<&String> = tool_names
        .iter()
        .filter(|name| !allowed.contains(name.as_str()))
        .collect();
    if disallowed.is_empty() {
        return Ok(());
    }
    Err(ToolCheckError::Denied(Box::new(tool_denial(&disallowed))))
}

impl ToolCheckError {
    #[must_use]
    pub fn into_proxy_exception(self) -> ProxyException {
        match self {
            Self::Denied(exception) => *exception,
            Self::Extraction(error) => error.into_proxy_exception(),
        }
    }
}

fn has_tool_capable_call_type(route: &str, extractors: &ToolNameExtractors) -> bool {
    extractors
        .routes
        .call_types_for_route(route)
        .iter()
        .any(|name| ToolCapableCallType::from_source_name(name).is_some())
}

// a nonempty key list wins over the team list, and an empty or malformed one falls back to it
fn effective_allowed_tools(token: &UserAPIKeyAuth) -> Option<&[JsonData]> {
    let key_metadata = &token.view.token.metadata.0;
    if let Some(entries) = nonempty_allowed_list(key_metadata.get("allowed_tools")) {
        return Some(entries);
    }
    let team_metadata = token.view.team_metadata.as_ref();
    team_metadata.and_then(|payload| nonempty_allowed_list(payload.0.get("allowed_tools")))
}

fn nonempty_allowed_list(value: Option<&JsonData>) -> Option<&[JsonData]> {
    match value {
        Some(JsonData::Array(entries)) if !entries.is_empty() => Some(entries),
        Some(
            JsonData::Array(_)
            | JsonData::Null
            | JsonData::Bool(_)
            | JsonData::Number(_)
            | JsonData::String(_)
            | JsonData::Object(_),
        )
        | None => None,
    }
}

// str(entry): the allowlist is whatever text the stored value turns into
fn allowed_string(value: &JsonData) -> String {
    match value {
        JsonData::Null => "None".to_owned(),
        JsonData::Bool(true) => "True".to_owned(),
        JsonData::Bool(false) => "False".to_owned(),
        JsonData::Number(number) => number.to_string(),
        JsonData::String(text) => text.clone(),
        JsonData::Array(_) | JsonData::Object(_) => {
            serde_json::to_string(value).unwrap_or_default()
        }
    }
}

fn tool_denial(disallowed: &[&String]) -> ProxyException {
    ProxyException::new(ProxyExceptionInit {
        message: format!(
            "Tool(s) {disallowed:?} are not in the allowed tools list for this key/team."
        ),
        error_type: ProxyErrorTypes::ToolAccessDenied.to_string(),
        param: Some(TOOL_DENIAL_PARAM.to_owned()),
        code: ExceptionCode::Status(TOOL_DENIAL_STATUS),
        headers: BTreeMap::new(),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    })
}

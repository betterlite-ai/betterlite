use std::collections::BTreeMap;

use betterlite_error::proxy_error_types::ProxyErrorTypes;
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rust_decimal::Decimal;
use thiserror::Error;

use crate::auth_key_budgets::key_budget::{BUDGET_EXCEEDED_STATUS, Denial};
use crate::auth_key_budgets::money::Money;
use crate::auxiliary_routes;
use crate::route_match::classifier::is_llm_api_route;
use crate::route_match::templates::{RouteValue, check_route_access};

// what the request-level checks read: the HTTP method, the route and the decoded body
#[derive(Debug)]
pub struct RequestFacts<'request> {
    pub method: Option<&'request str>,
    pub route: &'request str,
    pub body: &'request JsonObject,
}

// _enforce_user_param_check raises a bare Exception, which B757 answers as an ordinary 401
#[derive(Debug, Error, PartialEq, Eq)]
#[error("'user' param not passed in. 'enforce_user_param'=true")]
pub struct UserParamError;

pub fn enforce_user_param_check(
    enforce_user_param: bool,
    request: &RequestFacts<'_>,
) -> Result<(), UserParamError> {
    if !enforce_user_param {
        return Ok(());
    }
    let is_post = request
        .method
        .is_some_and(|method| method.to_uppercase() == "POST");
    if is_post
        && is_llm_api_route(&RouteValue::Text(request.route))
        && !is_mcp_route(request.route)
        && !request.body.contains_key("user")
    {
        return Err(UserParamError);
    }
    Ok(())
}

// an MCP route is either listed verbatim or matched by one of its route patterns
fn is_mcp_route(route: &str) -> bool {
    auxiliary_routes::MCP_ROUTES.contains(&route)
        || check_route_access(route, auxiliary_routes::MCP_ROUTES)
}

// _reject_clientside_metadata_tags_check: key presence is enough, a null or empty tags value is rejected
pub fn reject_clientside_metadata_tags_check(
    reject_clientside_metadata_tags: bool,
    request: &RequestFacts<'_>,
) -> Result<(), Box<ProxyException>> {
    if !reject_clientside_metadata_tags || !is_llm_api_route(&RouteValue::Text(request.route)) {
        return Ok(());
    }
    let Some(Json::Object(metadata)) = request.body.get("metadata") else {
        return Ok(());
    };
    if !metadata.contains_key("tags") {
        return Ok(());
    }
    Err(Box::new(metadata_tags_exception()))
}

fn metadata_tags_exception() -> ProxyException {
    ProxyException::new(ProxyExceptionInit {
        message: "Client-side 'metadata.tags' not allowed in request. 'reject_clientside_metadata_tags'=true. Tags can only be set via API key metadata.".to_owned(),
        error_type: ProxyErrorTypes::BadRequestError.to_string(),
        param: Some("metadata.tags".to_owned()),
        code: ExceptionCode::Status(400_i64),
        headers: BTreeMap::new(),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    })
}

// _global_proxy_budget_check: only a positive finite ceiling, a known spend, enabled checks and an LLM route can deny
#[must_use]
pub fn global_proxy_budget_check(
    max_budget: Money,
    global_spend: Option<Decimal>,
    skip_budget_checks: bool,
    route: &str,
) -> Option<Denial> {
    let Some(spend) = global_spend else {
        return None;
    };
    if skip_budget_checks
        || !is_llm_api_route(&RouteValue::Text(route))
        || route == "/v1/models"
        || route == "/models"
    {
        return None;
    }
    let Money::Finite(ceiling) = max_budget else {
        return None;
    };
    if ceiling <= Decimal::ZERO || spend <= ceiling {
        return None;
    }
    Some(Denial {
        current_cost: spend,
        max_budget: ceiling,
        entity_type: "proxy".to_owned(),
        entity_id: None,
        message: format!("Budget has been exceeded! Current cost: {spend}, Max budget: {ceiling}"),
        status_code: BUDGET_EXCEEDED_STATUS,
    })
}

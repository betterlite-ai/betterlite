use crate::helpers::Helper;
use betterlite_auth::route_match::classifier::{
    is_azure_openai_route, is_llm_api_route, is_mcp_discovery_route,
};
use betterlite_auth::route_match::groups::get_actual_routes;
use betterlite_auth::route_match::method::{MethodAccess, get_request_method};
use betterlite_auth::route_match::templates::{
    RouteValue, check_route_access, route_matches_allowed_route, route_matches_pattern,
    route_matches_wildcard_pattern,
};
use betterlite_auth::{auxiliary_routes, management_routes};
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", deny_unknown_fields)]
pub(crate) enum Input {
    #[serde(rename = "get_actual_routes")]
    GetActualRoutes { routes: Option<Vec<String>> },
    #[serde(rename = "get_request_method")]
    GetRequestMethod { method: Option<String> },
    #[serde(rename = "is_llm_api_route")]
    IsLlmApiRoute { route: Option<Value> },
    #[serde(rename = "is_azure_openai_route")]
    IsAzureOpenaiRoute { route: Option<Value> },
    #[serde(rename = "is_management_route")]
    IsManagementRoute { route: Option<Value> },
    #[serde(rename = "is_info_route")]
    IsInfoRoute { route: Option<Value> },
    #[serde(rename = "is_mcp_discovery_route")]
    IsMcpDiscoveryRoute {
        route: Option<Value>,
        method: Option<String>,
    },
    #[serde(rename = "route_matches_pattern")]
    RouteMatchesPattern {
        route: Option<Value>,
        pattern: Option<String>,
    },
    #[serde(rename = "is_wildcard_pattern")]
    IsWildcardPattern { pattern: Option<String> },
    #[serde(rename = "route_matches_wildcard_pattern")]
    RouteMatchesWildcardPattern {
        route: Option<Value>,
        pattern: Option<String>,
    },
    #[serde(rename = "route_matches_allowed_route")]
    RouteMatchesAllowedRoute {
        route: Option<Value>,
        allowed_route: Option<String>,
    },
    #[serde(rename = "check_route_access")]
    CheckRouteAccess {
        route: Option<Value>,
        allowed_routes: Option<Vec<String>>,
    },
}

enum Operation<'input> {
    ActualRoutes(Option<&'input [String]>),
    RequestMethod(Option<&'input str>),
    Bool(BoolOp<'input>),
}

enum BoolOp<'input> {
    LlmRoute(Option<&'input str>),
    AzureRoute(Option<&'input str>),
    ManagementRoute(Option<&'input str>),
    InfoRoute(Option<&'input str>),
    McpDiscovery(Option<&'input str>, Option<&'input str>),
    Pattern(Option<&'input str>, Option<&'input str>),
    WildcardPattern(Option<&'input str>),
    WildcardMatch(Option<&'input str>, Option<&'input str>),
    AllowedRoute(Option<&'input str>, Option<&'input str>),
    RouteAccess(Option<&'input str>, Option<&'input [String]>),
}

impl Input {
    fn operation(&self) -> Operation<'_> {
        match self {
            Self::GetActualRoutes { routes } => Operation::ActualRoutes(routes.as_deref()),
            Self::GetRequestMethod { method } => Operation::RequestMethod(method.as_deref()),
            Self::IsLlmApiRoute { route } => {
                Operation::Bool(BoolOp::LlmRoute(route_text(route.as_ref())))
            }
            Self::IsAzureOpenaiRoute { route } => {
                Operation::Bool(BoolOp::AzureRoute(route_text(route.as_ref())))
            }
            Self::IsManagementRoute { route } => {
                Operation::Bool(BoolOp::ManagementRoute(route_text(route.as_ref())))
            }
            Self::IsInfoRoute { route } => {
                Operation::Bool(BoolOp::InfoRoute(route_text(route.as_ref())))
            }
            Self::IsMcpDiscoveryRoute { route, method } => Operation::Bool(BoolOp::McpDiscovery(
                route_text(route.as_ref()),
                method.as_deref(),
            )),
            Self::RouteMatchesPattern { route, pattern } => Operation::Bool(BoolOp::Pattern(
                route_text(route.as_ref()),
                pattern.as_deref(),
            )),
            Self::IsWildcardPattern { pattern } => {
                Operation::Bool(BoolOp::WildcardPattern(pattern.as_deref()))
            }
            Self::RouteMatchesWildcardPattern { route, pattern } => Operation::Bool(
                BoolOp::WildcardMatch(route_text(route.as_ref()), pattern.as_deref()),
            ),
            Self::RouteMatchesAllowedRoute {
                route,
                allowed_route,
            } => Operation::Bool(BoolOp::AllowedRoute(
                route_text(route.as_ref()),
                allowed_route.as_deref(),
            )),
            Self::CheckRouteAccess {
                route,
                allowed_routes,
            } => Operation::Bool(BoolOp::RouteAccess(
                route_text(route.as_ref()),
                allowed_routes.as_deref(),
            )),
        }
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input.operation() {
            Operation::ActualRoutes(routes) => actual_routes(routes),
            Operation::RequestMethod(method) => request_method(method),
            Operation::Bool(operation) => Ok(Value::Bool(boolean_result(&operation))),
        }
    }
}

fn route_text(route: Option<&Value>) -> Option<&str> {
    match route {
        Some(Value::String(path)) => Some(path),
        Some(_) | None => None,
    }
}

fn boolean_result(operation: &BoolOp<'_>) -> bool {
    match operation {
        BoolOp::LlmRoute(route) => {
            is_llm_api_route(&route.map_or(RouteValue::NonString, RouteValue::Text))
        }
        BoolOp::AzureRoute(route) => {
            is_azure_openai_route(&route.map_or(RouteValue::NonString, RouteValue::Text))
        }
        BoolOp::ManagementRoute(Some(path)) => {
            check_route_access(path, &management_routes::management_routes())
        }
        BoolOp::InfoRoute(Some(path)) => auxiliary_routes::INFO_ROUTES.contains(path),
        BoolOp::McpDiscovery(Some(path), method) => is_mcp_discovery_route(path, *method),
        BoolOp::Pattern(Some(path), Some(expression)) => {
            route_matches_pattern(&RouteValue::Text(path), expression)
        }
        BoolOp::WildcardPattern(Some(expression)) => {
            matches!(expression.chars().last(), Some('*'))
        }
        BoolOp::WildcardMatch(Some(path), Some(expression)) => {
            route_matches_wildcard_pattern(path, expression)
        }
        BoolOp::AllowedRoute(Some(path), Some(allowed)) => {
            route_matches_allowed_route(path, allowed)
        }
        BoolOp::RouteAccess(Some(path), Some(allowed)) => check_route_access(
            path,
            &allowed.iter().map(String::as_str).collect::<Vec<_>>(),
        ),
        BoolOp::ManagementRoute(None)
        | BoolOp::InfoRoute(None)
        | BoolOp::McpDiscovery(None, _)
        | BoolOp::Pattern(_, _)
        | BoolOp::WildcardPattern(None)
        | BoolOp::WildcardMatch(_, _)
        | BoolOp::AllowedRoute(_, _)
        | BoolOp::RouteAccess(_, _) => false,
    }
}

fn actual_routes(routes: Option<&[String]>) -> crate::helpers::HelperResult {
    let references: Vec<&str> = routes
        .unwrap_or_default()
        .iter()
        .map(String::as_str)
        .collect();
    serde_json::to_value(get_actual_routes(&references))
        .map_err(|error| helper_error("SerializationError", error.to_string()))
}

fn request_method(method: Option<&str>) -> crate::helpers::HelperResult {
    let access = method.map_or(MethodAccess::NoRequest, MethodAccess::Text);
    let parsed_method = get_request_method(access)
        .map_err(|error| helper_error("MethodError", error.to_string()))?;
    serde_json::to_value(parsed_method)
        .map_err(|error| helper_error("SerializationError", error.to_string()))
}

fn helper_error(class: &str, message: String) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

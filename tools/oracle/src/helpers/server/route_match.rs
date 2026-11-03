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

enum BoolOp<'input> {
    McpDiscovery(Option<&'input str>, Option<&'input str>),
    Pattern(Option<&'input str>, Option<&'input str>),
    WildcardPattern(Option<&'input str>),
    WildcardMatch(Option<&'input str>, Option<&'input str>),
    AllowedRoute(Option<&'input str>, Option<&'input str>),
    RouteAccess(Option<&'input str>, Option<&'input [String]>),
}

enum RouteClassifier {
    Llm,
    Azure,
    Management,
    Info,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::GetActualRoutes { routes } => actual_routes(routes.as_deref()),
            Input::GetRequestMethod { method } => request_method(method.as_deref()),
            Input::IsLlmApiRoute { route } => Ok(Value::Bool(classifier_result(
                &RouteClassifier::Llm,
                route_text(route.as_ref()),
            ))),
            Input::IsAzureOpenaiRoute { route } => Ok(Value::Bool(classifier_result(
                &RouteClassifier::Azure,
                route_text(route.as_ref()),
            ))),
            Input::IsManagementRoute { route } => Ok(Value::Bool(classifier_result(
                &RouteClassifier::Management,
                route_text(route.as_ref()),
            ))),
            Input::IsInfoRoute { route } => Ok(Value::Bool(classifier_result(
                &RouteClassifier::Info,
                route_text(route.as_ref()),
            ))),
            Input::IsMcpDiscoveryRoute { route, method } => Ok(Value::Bool(boolean_result(
                &BoolOp::McpDiscovery(route_text(route.as_ref()), method.as_deref()),
            ))),
            Input::RouteMatchesPattern { route, pattern } => Ok(Value::Bool(boolean_result(
                &BoolOp::Pattern(route_text(route.as_ref()), pattern.as_deref()),
            ))),
            Input::IsWildcardPattern { pattern } => Ok(Value::Bool(boolean_result(
                &BoolOp::WildcardPattern(pattern.as_deref()),
            ))),
            Input::RouteMatchesWildcardPattern { route, pattern } => {
                Ok(Value::Bool(boolean_result(&BoolOp::WildcardMatch(
                    route_text(route.as_ref()),
                    pattern.as_deref(),
                ))))
            }
            Input::RouteMatchesAllowedRoute {
                route,
                allowed_route,
            } => Ok(Value::Bool(boolean_result(&BoolOp::AllowedRoute(
                route_text(route.as_ref()),
                allowed_route.as_deref(),
            )))),
            Input::CheckRouteAccess {
                route,
                allowed_routes,
            } => Ok(Value::Bool(boolean_result(&BoolOp::RouteAccess(
                route_text(route.as_ref()),
                allowed_routes.as_deref(),
            )))),
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
        BoolOp::McpDiscovery(None, _)
        | BoolOp::Pattern(_, _)
        | BoolOp::WildcardPattern(None)
        | BoolOp::WildcardMatch(_, _)
        | BoolOp::AllowedRoute(_, _)
        | BoolOp::RouteAccess(_, _) => false,
    }
}

fn classifier_result(classifier: &RouteClassifier, route: Option<&str>) -> bool {
    match (classifier, route) {
        (RouteClassifier::Llm, Some(path)) => is_llm_api_route(&RouteValue::Text(path)),
        (RouteClassifier::Azure, Some(path)) => is_azure_openai_route(&RouteValue::Text(path)),
        (RouteClassifier::Management, Some(path)) => {
            check_route_access(path, &management_routes::management_routes())
        }
        (RouteClassifier::Info, Some(path)) => auxiliary_routes::INFO_ROUTES.contains(&path),
        (_, None) => false,
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

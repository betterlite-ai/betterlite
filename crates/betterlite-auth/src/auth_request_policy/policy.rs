use std::collections::BTreeMap;

use betterlite_config::sections::auth_request_policy::Settings;
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};
use thiserror::Error;

use crate::auth_request_policy::bytes::{SizeError, check_request_size};
use crate::auth_request_policy::ip::{RequestProvenance, check_valid_ip};
use crate::request_body_safety::{RequestBodySafety, SafetyError, is_request_body_safe};

const AUTH_ERROR: &str = "auth_error";
const PARAM_NONE: &str = "None";
const FORBIDDEN: i64 = 403;

#[derive(Debug, Error)]
pub enum PolicyError {
    #[error(transparent)]
    Size(#[from] SizeError),
    #[error(transparent)]
    BodySafety(#[from] SafetyError),
    #[error("Access forbidden: IP address {client_ip} not allowed.")]
    IpDenied { client_ip: String },
    #[error("Access forbidden: Route {route} not allowed")]
    RouteDenied { route: String },
}

impl PolicyError {
    #[must_use]
    pub fn into_proxy_exception(self) -> ProxyException {
        match self {
            Self::Size(error) => error.into_proxy_exception(),
            Self::BodySafety(error) => error.into_proxy_exception(),
            Self::IpDenied { client_ip } => forbidden(format!(
                "Access forbidden: IP address {client_ip} not allowed."
            )),
            Self::RouteDenied { route } => {
                forbidden(format!("Access forbidden: Route {route} not allowed"))
            }
        }
    }
}

fn forbidden(message: String) -> ProxyException {
    ProxyException::new(ProxyExceptionInit {
        message,
        error_type: AUTH_ERROR.to_owned(),
        param: Some(PARAM_NONE.to_owned()),
        code: ExceptionCode::Status(FORBIDDEN),
        headers: BTreeMap::new(),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RouteDecision {
    Allowed,
    Denied,
}

#[must_use]
pub fn check_allowed_route(reference: Option<&[String]>, route: &str) -> RouteDecision {
    let Some(allowed_routes) = reference else {
        return RouteDecision::Allowed;
    };
    if allowed_routes
        .iter()
        .any(|candidate| candidate.as_str() == route)
    {
        RouteDecision::Allowed
    } else {
        RouteDecision::Denied
    }
}

#[derive(Debug)]
pub struct PreDbRequest<'scan> {
    pub content_length: Option<&'scan str>,
    pub body: &'scan [u8],
    pub route: &'scan str,
    pub provenance: RequestProvenance<'scan>,
    pub body_safety: Option<RequestBodySafety<'scan>>,
}

pub fn pre_db_read_auth_checks(
    request: PreDbRequest<'_>,
    settings: &Settings,
) -> Result<(), PolicyError> {
    check_request_size(
        request.content_length,
        request.body,
        settings.max_request_size_mb,
    )?;
    if let Some(safety) = &request.body_safety {
        is_request_body_safe(safety)?;
    }
    if let Some(allowed_ips) = settings.allowed_ips.as_deref() {
        let verdict = check_valid_ip(
            Some(allowed_ips),
            &request.provenance,
            settings.use_x_forwarded_for,
        );
        if !verdict.allowed {
            let client_ip = verdict.passed_in_ip.unwrap_or_default().to_owned();
            return Err(PolicyError::IpDenied { client_ip });
        }
    }
    if let Some(allowed_routes) = settings.allowed_routes.as_deref()
        && check_allowed_route(Some(allowed_routes), request.route) == RouteDecision::Denied
    {
        tracing::error!(
            route = request.route,
            allowed_routes = ?allowed_routes,
            "the request route is not in allowed_routes"
        );
        return Err(PolicyError::RouteDenied {
            route: request.route.to_owned(),
        });
    }
    Ok(())
}

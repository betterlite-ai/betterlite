use std::collections::BTreeMap;

use betterlite_db::database_availability::chain::Chain;
use betterlite_db::database_availability::classification::is_service_unavailable;
use betterlite_db::database_availability::projection::service_unavailable_error;
use betterlite_db::database_faults::{DbError, is_database_connection_error};
use betterlite_error::error::details::BudgetExceededDetails;
use betterlite_error::error::{ErrorClass as BudgetErrorClass, class_message};
use betterlite_error::proxy_error_types::ProxyErrorTypes;
// ast-grep-ignore: no-long-names
use betterlite_error::proxy_exception::model_access_denied::ModelAccessDeniedProxyException;
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};
use betterlite_models::call_helpers::expected_error::ClientErrorFacts;
use betterlite_models::decisions::Json;
use serde_json::Number;

use crate::auth_request_policy::invalid_key::{ErrorClass, ErrorFacts, Marker};

// HTTPException and an untyped exception both read their param with getattr(e, "param", "None")
const ABSENT_PARAM: &str = "None";
const UNAUTHORIZED: u16 = 401;
// getattr(e, "status_code", 429) when a BudgetExceededError lost its status_code attribute
const BUDGET_FALLBACK_STATUS: u16 = 429;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AuthFailure {
    #[error("{message}")]
    BudgetExceeded {
        message: String,
        status_code: Option<u16>,
        llm_provider: String,
    },
    // Starlette's HTTPException prints as "status: detail"
    #[error("{status_code}: {detail}")]
    ModelAccessDenied {
        detail: String,
        internal_message: String,
        status_code: u16,
    },
    #[error("{status_code}: {detail}")]
    Http {
        detail: String,
        param: Option<String>,
        status_code: u16,
        invalid_virtual_key: bool,
    },
    #[error("{exception}")]
    Proxy {
        exception: ProxyException,
        invalid_virtual_key: bool,
    },
    #[error("{0}")]
    Database(DbError),
    #[error("{message}")]
    Other {
        message: String,
        param: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientFailure {
    Proxy(ProxyException),
    // ast-grep-ignore: no-long-names
    ModelAccessDenied(ModelAccessDeniedProxyException),
}

impl ClientFailure {
    #[must_use]
    pub const fn exception(&self) -> &ProxyException {
        match self {
            Self::Proxy(exception) => exception,
            Self::ModelAccessDenied(denial) => denial.as_proxy_exception(),
        }
    }
}

impl AuthFailure {
    #[must_use]
    pub fn marker_facts(&self) -> ErrorFacts<'_> {
        let marker_for = |marked: bool| {
            if marked {
                Marker::ExactTrue
            } else {
                Marker::Absent
            }
        };
        match self {
            Self::Http {
                status_code,
                invalid_virtual_key,
                ..
            } => http_facts(*status_code, marker_for(*invalid_virtual_key)),
            Self::ModelAccessDenied { status_code, .. } => http_facts(*status_code, Marker::Absent),
            Self::Proxy {
                exception,
                invalid_virtual_key,
            } => ErrorFacts {
                marker: marker_for(*invalid_virtual_key),
                ..ErrorFacts::proxy(exception)
            },
            Self::BudgetExceeded { .. } | Self::Database(_) | Self::Other { .. } => ErrorFacts {
                class: ErrorClass::Other,
                code: None,
                status_code: None,
                marker: Marker::Absent,
            },
        }
    }

    // PrismaDBExceptionHandler.is_database_connection_error also counts the proxy's own no-database rejection
    #[must_use]
    pub fn is_database_connection_error(&self) -> bool {
        match self {
            Self::Database(error) => is_database_connection_error(error.error_type),
            Self::Proxy { exception, .. } => {
                exception.error_type == ProxyErrorTypes::NoDbConnection.to_string()
            }
            Self::BudgetExceeded { .. }
            | Self::ModelAccessDenied { .. }
            | Self::Http { .. }
            | Self::Other { .. } => false,
        }
    }

    #[must_use]
    pub fn client_error_facts(&self) -> ClientErrorFacts {
        let (code, status_code, http_exception) = match self {
            Self::BudgetExceeded { status_code, .. } => (None, *status_code, false),
            Self::ModelAccessDenied { status_code, .. } | Self::Http { status_code, .. } => {
                (None, Some(*status_code), true)
            }
            Self::Proxy { exception, .. } => {
                (Some(Json::Text(exception.code.clone())), None, false)
            }
            Self::Database(_) | Self::Other { .. } => (None, None, false),
        };
        let (llm_provider, category) = match self {
            Self::BudgetExceeded { llm_provider, .. } => (
                Some(llm_provider.clone()),
                Some("litellm_rate_limit".to_owned()),
            ),
            Self::ModelAccessDenied { .. }
            | Self::Http { .. }
            | Self::Proxy { .. }
            | Self::Database(_)
            | Self::Other { .. } => (None, None),
        };
        ClientErrorFacts {
            code,
            status_code: status_code.map(|status| Json::Number(Number::from(status))),
            llm_provider,
            category,
            http_exception,
            base_llm_exception: false,
        }
    }
}

const fn http_facts(status_code: u16, marker: Marker) -> ErrorFacts<'static> {
    ErrorFacts {
        class: ErrorClass::Http,
        code: None,
        status_code: Some(status_code),
        marker,
    }
}

#[must_use]
pub fn as_proxy_exception(failure: &AuthFailure) -> ClientFailure {
    match failure {
        AuthFailure::BudgetExceeded {
            message,
            status_code,
            ..
        } => ClientFailure::Proxy(budget_exception(message, *status_code)),
        AuthFailure::ModelAccessDenied {
            detail,
            internal_message,
            status_code,
        } => model_access_denied(detail, internal_message, *status_code),
        AuthFailure::Http {
            detail,
            param,
            status_code,
            ..
        } => ClientFailure::Proxy(http_exception(detail, param.as_deref(), *status_code)),
        AuthFailure::Proxy { exception, .. } => ClientFailure::Proxy(exception.clone()),
        AuthFailure::Database(error) => database_exception(error),
        AuthFailure::Other { message, param } => {
            ClientFailure::Proxy(other_exception(message, param.as_deref()))
        }
    }
}

// BudgetExceededError fills an absent or empty message with the costs, and the wire keeps the rejection's status
#[must_use]
pub fn budget_details_exception(details: &BudgetExceededDetails) -> ProxyException {
    let class = BudgetErrorClass::BudgetExceeded(Box::new(details.clone()));
    let failure = AuthFailure::BudgetExceeded {
        message: class_message(&class),
        status_code: Some(details.status_code),
        llm_provider: details.provider.clone(),
    };
    as_proxy_exception(&failure).exception().clone()
}

// an exception that is not a budget rejection reaches the wire as an auth error with a null param
#[must_use]
pub fn other_message_exception(message: &str) -> ProxyException {
    let failure = AuthFailure::Other {
        message: message.to_owned(),
        param: None,
    };
    as_proxy_exception(&failure).exception().clone()
}

fn budget_exception(message: &str, status_code: Option<u16>) -> ProxyException {
    proxy_exception(
        message.to_owned(),
        ProxyErrorTypes::BudgetExceeded.to_string(),
        None,
        status_code.unwrap_or(BUDGET_FALLBACK_STATUS),
    )
}

fn http_exception(detail: &str, param: Option<&str>, status_code: u16) -> ProxyException {
    proxy_exception(
        detail.to_owned(),
        ProxyErrorTypes::AuthError.to_string(),
        Some(param.unwrap_or(ABSENT_PARAM).to_owned()),
        status_code,
    )
}

fn other_exception(message: &str, param: Option<&str>) -> ProxyException {
    proxy_exception(
        format!("Authentication Error, {message}"),
        ProxyErrorTypes::AuthError.to_string(),
        Some(param.unwrap_or(ABSENT_PARAM).to_owned()),
        UNAUTHORIZED,
    )
}

fn model_access_denied(detail: &str, internal_message: &str, status_code: u16) -> ClientFailure {
    // ast-grep-ignore: no-long-names
    ClientFailure::ModelAccessDenied(ModelAccessDeniedProxyException::new(
        detail.to_owned(),
        internal_message.to_owned(),
        ProxyErrorTypes::AuthError.to_string(),
        Some(ABSENT_PARAM.to_owned()),
        ExceptionCode::Status(i64::from(status_code)),
    ))
}

// B1217 answers an unreachable database with its 503; any other database error is an ordinary 401
fn database_exception(error: &DbError) -> ClientFailure {
    let chain = Chain::from_error(error);
    if !chain.failures.first().is_some_and(is_service_unavailable) {
        return ClientFailure::Proxy(proxy_exception(
            format!("Authentication Error, {error}"),
            ProxyErrorTypes::AuthError.to_string(),
            Some(ABSENT_PARAM.to_owned()),
            UNAUTHORIZED,
        ));
    }
    let unavailable = service_unavailable_error(&chain);
    ClientFailure::Proxy(proxy_exception(
        unavailable.message,
        unavailable.error_type.to_owned(),
        Some(unavailable.param.to_owned()),
        unavailable.status,
    ))
}

fn proxy_exception(
    message: String,
    error_type: String,
    param: Option<String>,
    status_code: u16,
) -> ProxyException {
    ProxyException::new(ProxyExceptionInit {
        message,
        error_type,
        param,
        code: ExceptionCode::Status(i64::from(status_code)),
        headers: BTreeMap::new(),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    })
}

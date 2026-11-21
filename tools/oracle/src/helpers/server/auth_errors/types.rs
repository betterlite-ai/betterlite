use std::collections::BTreeMap;

use betterlite_auth::auth_errors::conversion::AuthFailure;
use betterlite_db::database_faults::{DbError, ErrorType};
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};
use betterlite_models::model_registration::JsonObject;
use serde::Deserialize;

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Http,
    Proxy,
    ModelDenial,
    Connect,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Failure {
    kind: Kind,
    message: String,
    #[serde(rename = "type")]
    error_type: String,
    param: Option<String>,
    status_code: u16,
    marked: bool,
    internal_message: String,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum Switch {
    AllowRequestsOnDbUnavailable,
    LogClientErrorTracebacks,
    LogAuthFailureKeyIdentity,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Reply {
    Original,
    Replace,
    Raise,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Row {
    pub(super) failure: Failure,
    pub(super) reply: Reply,
    pub(super) replacement: Option<Failure>,
    pub(super) switches: Vec<Switch>,
    pub(super) identity: Option<BTreeMap<String, String>>,
    pub(super) request_data: JsonObject,
    pub(super) route: String,
    pub(super) user_agent: Option<String>,
    pub(super) client_host: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(super) rows: Vec<Row>,
}

pub(super) fn auth_failure(spec: &Failure) -> AuthFailure {
    let message = spec.message.clone();
    match spec.kind {
        Kind::Http => AuthFailure::Http {
            detail: message,
            param: None,
            status_code: spec.status_code,
            invalid_virtual_key: spec.marked,
        },
        Kind::Proxy => AuthFailure::Proxy {
            exception: ProxyException::new(ProxyExceptionInit {
                message,
                error_type: spec.error_type.clone(),
                param: spec.param.clone(),
                code: ExceptionCode::Status(i64::from(spec.status_code)),
                headers: BTreeMap::new(),
                openai_code: None,
                provider_specific_fields: BTreeMap::new(),
            }),
            invalid_virtual_key: spec.marked,
        },
        Kind::ModelDenial => AuthFailure::ModelAccessDenied {
            detail: message,
            internal_message: spec.internal_message.clone(),
            status_code: spec.status_code,
        },
        Kind::Connect => AuthFailure::Database(DbError {
            error_type: ErrorType::Connect,
            message,
            code: None,
            meta_code: None,
            traceback_modules: Vec::new(),
            cause: None,
        }),
    }
}

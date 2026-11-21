use std::collections::BTreeMap;
use std::error::Error;
use std::sync::Mutex;

use axum::http::header::USER_AGENT;
use axum::http::{HeaderMap, HeaderValue};
use betterlite_auth::auth_errors::conversion::ClientFailure;
use betterlite_auth::auth_errors::handler::{
    AuthErrorHandler, AuthErrorSettings, FailedRequest, HandlerError, LogSwitches,
};
use betterlite_auth::auth_errors::request_state::record_unparsable_body_failure;
use betterlite_auth::auth_request_policy::ip::RequestProvenance;
use betterlite_auth::auth_route_patterns::normalize::RouteNormalizer;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::runtime::Builder;

use self::recorder::Recorder;
use self::types::{Input, Row, Step, Switch, auth_failure, proxy_exception};

mod recorder;
mod tests;
mod types;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .build()
            .map_err(|error| failure("RuntimeError", &error))?;
        let routes = RouteNormalizer::new().map_err(|error| failure("NormalizeError", &error))?;
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
            .map_err(|error| failure("RedactionError", &error))?;
        let mut answers = Vec::new();
        for row in &input.rows {
            answers.push(runtime.block_on(answer(row, &routes, &redactor))?);
        }
        Ok(Value::Array(answers))
    }
}

async fn answer(row: &Row, routes: &RouteNormalizer, redactor: &Redactor) -> HelperResult {
    let ports = Recorder {
        events: Mutex::new(Vec::new()),
        reply: row.reply,
        replacement: row.replacement.as_ref().map(auth_failure),
    };
    let resolved = row.identity.as_ref().map(identity).transpose()?;
    if matches!(row.step, Step::Unparsable) {
        return unparsable(row, ports, resolved).await;
    }
    let mut headers = HeaderMap::new();
    if let Some(agent) = &row.user_agent {
        let value = HeaderValue::from_str(agent).map_err(|error| failure("HeaderError", &error))?;
        headers.insert(USER_AGENT, value);
    }
    let api_key = SecretString::from("sk-presented-key".to_owned());
    let request = FailedRequest {
        request_data: &row.request_data,
        route: &row.route,
        parent_span: None,
        api_key: &api_key,
        resolved_identity: resolved.as_ref(),
        provenance: RequestProvenance {
            forwarded_for: None,
            client_host: Some(&row.client_host),
        },
        scope_headers: Some(&headers),
    };
    let handler = AuthErrorHandler {
        ports: &ports,
        settings: settings(&row.switches),
        routes,
        redactor,
    };
    let result = outcome(handler.handle(auth_failure(&row.failure), request).await)?;
    let events = ports.events.into_inner().unwrap_or_default();
    Ok(json!({"outcome": result, "events": events}))
}

async fn unparsable(row: &Row, ports: Recorder, resolved: Option<UserAPIKeyAuth>) -> HelperResult {
    let caller = match resolved {
        Some(known) => known,
        None => identity(&BTreeMap::new())?,
    };
    let exception = proxy_exception(&row.failure);
    record_unparsable_body_failure(&ports, &caller, &exception, &row.route).await;
    let events = ports.events.into_inner().unwrap_or_default();
    Ok(json!({"outcome": {"recorded": true}, "events": events}))
}

fn outcome(handled: Result<UserAPIKeyAuth, HandlerError>) -> HelperResult {
    Ok(match handled {
        Ok(granted) => json!({"identity": {"user_id": field(&granted, "user_id"),
            "key_name": field(&granted, "key_name"), "user_role": field(&granted, "user_role"),
            "request_route": field(&granted, "request_route")}}),
        Err(HandlerError::Rejected(raised)) => json!({"raised": wire(&raised.failure)}),
        Err(HandlerError::Hook(_)) => json!({"hook_error": true}),
        Err(HandlerError::Identity(error)) => return Err(failure("AuthError", &error)),
    })
}

fn settings(switches: &[Switch]) -> AuthErrorSettings {
    AuthErrorSettings {
        log: LogSwitches {
            client_error_tracebacks: switches.contains(&Switch::LogClientErrorTracebacks),
            key_identity: switches.contains(&Switch::LogAuthFailureKeyIdentity),
        },
        allow_requests_on_db_unavailable: switches.contains(&Switch::AllowRequestsOnDbUnavailable),
        use_x_forwarded_for: false,
    }
}

fn identity(given: &BTreeMap<String, String>) -> Result<UserAPIKeyAuth, Box<HelperError>> {
    let dict = given
        .iter()
        .map(|(name, value)| (name.clone(), JsonData::String(value.clone())))
        .collect();
    UserAPIKeyAuth::construct(AuthInput::Dict(dict)).map_err(|error| failure("AuthError", &error))
}

// the identity's fields are crate-private, so the answer reads them from its serialized form
fn field(identity: &UserAPIKeyAuth, name: &str) -> Value {
    let whole = serde_json::to_value(identity).unwrap_or_default();
    whole.get(name).cloned().unwrap_or_default()
}

fn wire(raised: &ClientFailure) -> Value {
    let exception = raised.exception();
    let internal_message = match raised {
        ClientFailure::ModelAccessDenied(denial) => Some(denial.sanitized_internal_message()),
        ClientFailure::Proxy(_) => None,
    };
    json!({"message": exception.message, "type": exception.error_type, "param": exception.param,
        "code": exception.code, "internal_message": internal_message})
}

fn failure(class: &str, error: &dyn Error) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

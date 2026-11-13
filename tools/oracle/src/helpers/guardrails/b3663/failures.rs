use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_message_scope::unappliable_rewrite::UnappliableRequestRewrite;
use betterlite_server::guardrail_message_scope::exception::{
    FastapiInstall, GuardrailCallback, GuardrailRaisedException, HttpException, RaisedError,
    add_guardrail_context, is_fastapi_http_exception, pre_call_rejection,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{HelperError, HelperResult};

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Fastapi {
    Installed,
    Absent,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Raised {
    Http,
    Guardrail,
    Other,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct IsHttpException {
    fastapi: Fastapi,
    raised: Raised,
    status_code: u16,
    block_status_codes: Vec<u16>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EnrichException {
    fastapi: Fastapi,
    raised: Raised,
    detail: GuardrailValue,
    guardrail_name: Option<String>,
    event_hook: GuardrailValue,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PreCall {
    fastapi: Fastapi,
    message: String,
    guardrail_name: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Unappliable {
    guardrail_name: Option<String>,
}

const fn install(fastapi: &Fastapi) -> FastapiInstall {
    match fastapi {
        Fastapi::Installed => FastapiInstall::Installed,
        Fastapi::Absent => FastapiInstall::Absent,
    }
}

fn raised_error(raised: &Raised, status_code: u16, detail: &GuardrailValue) -> RaisedError {
    match raised {
        Raised::Http => RaisedError::Http(HttpException {
            status_code,
            detail: detail.clone(),
        }),
        Raised::Guardrail => RaisedError::Guardrail(GuardrailRaisedException {
            guardrail_name: Some("guard".to_owned()),
            message: "blocked".to_owned(),
            status_code,
            blocked_content: false,
        }),
        Raised::Other => RaisedError::Other,
    }
}

pub(super) fn is_http_exception(request: &IsHttpException) -> Value {
    let error = raised_error(&request.raised, request.status_code, &GuardrailValue::Null);
    json!({
        "is_http": is_fastapi_http_exception(
            &error,
            &request.block_status_codes,
            install(&request.fastapi),
        ),
    })
}

pub(super) fn enrich_exception(request: &EnrichException) -> Value {
    let mut error = raised_error(&request.raised, 400, &request.detail);
    let callback = GuardrailCallback {
        guardrail_name: request.guardrail_name.clone(),
        event_hook: Some(request.event_hook.clone()),
    };
    add_guardrail_context(&mut error, &callback, install(&request.fastapi));
    match error {
        RaisedError::Http(http) => json!({"detail": http.detail}),
        RaisedError::Guardrail(_) | RaisedError::Other => json!({"detail": request.detail}),
    }
}

pub(super) fn pre_call(request: &PreCall) -> HelperResult {
    let error = pre_call_rejection(
        &request.message,
        request.guardrail_name.as_deref(),
        install(&request.fastapi),
    );
    match error {
        RaisedError::Http(http) => Ok(json!({
            "kind": "http",
            "status_code": http.status_code,
            "detail": http.detail,
        })),
        RaisedError::Guardrail(raised) => Ok(json!({
            "kind": "guardrail",
            "guardrail_name": raised.guardrail_name,
            "message": raised.message,
            "status_code": raised.status_code,
            "blocked_content": raised.blocked_content,
        })),
        RaisedError::Other => Err(Box::new(HelperError {
            class: "UnexpectedRejection".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: "pre_call_rejection returned a plain error".to_owned(),
        })),
    }
}

pub(super) fn unappliable(request: &Unappliable) -> Value {
    let error = UnappliableRequestRewrite::new(request.guardrail_name.as_deref());
    json!({
        "guardrail_name": error.guardrail_name,
        "message": error.to_string(),
    })
}

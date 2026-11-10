mod bodies;
mod capture;
mod debug;
mod failures;

use std::collections::BTreeMap;

use betterlite_http::user_url::httpx_url::HttpxUrl;
use betterlite_mcp::credential_contracts::auth_spec::AuthResolution;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_server::mcp::debug::mask_secret;
use betterlite_server::mcp::debug::preview::upstream::{
    BodyState, UpstreamRequest, UpstreamResponse,
};
use http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

use crate::helpers::{HelperError, HelperResult};

use super::input::{Input, MaskInput, Pair, RequestSpec, ResponseSpec};

pub(crate) fn run(input: &Input) -> HelperResult {
    match input {
        Input::DebugHeaders(value) => debug::debug_headers(value),
        Input::AuthDiagnostics(value) => debug::auth_diagnostics(value),
        Input::WrapSend(value) => debug::wrap_send(value),
        Input::MaskSecret(value) => Ok(masked(value)),
        Input::Preview(value) => bodies::previews(value),
        Input::RequestSecrets(value) => bodies::request_secrets(value),
        Input::DescribeUpstream(value) => bodies::describe(value),
        Input::DescribeFailure(value) => failures::describe_failure(value),
        Input::Capture(value) => capture::capture(value),
    }
}

fn masked(input: &MaskInput) -> Value {
    json!({
        "masks": input.values.iter().map(|value| mask_secret(value.as_deref())).collect::<Vec<_>>(),
    })
}

pub(super) fn mapped<Item, Output, Step>(items: &[Item], step: Step) -> Vec<Output>
where
    Step: Fn(&Item) -> Output,
{
    let mut output = Vec::with_capacity(items.len());
    for item in items {
        output.push(step(item));
    }
    output
}

pub(super) fn secret_map(pairs: &[Pair]) -> BTreeMap<String, SecretString> {
    let mut map = BTreeMap::new();
    for (name, value) in pairs {
        map.insert(name.clone(), SecretString::from(value.clone()));
    }
    map
}

pub(super) fn response_list(
    specs: &[ResponseSpec],
) -> Result<Vec<UpstreamResponse>, Box<HelperError>> {
    let mut responses = Vec::with_capacity(specs.len());
    for spec in specs {
        responses.push(upstream_response(spec, None)?);
    }
    Ok(responses)
}

fn body_state(body: Option<&String>) -> BodyState {
    body.map_or(BodyState::NotRead, |text| {
        BodyState::Read(text.as_bytes().to_vec())
    })
}

pub(super) fn upstream_request(spec: &RequestSpec) -> Result<UpstreamRequest, Box<HelperError>> {
    let url = HttpxUrl::parse(&spec.url)
        .map_err(|error| Box::new(failure("InvalidURL", error.to_string())))?;
    let headers = header_map(&spec.headers)?;
    let body = body_state(spec.body.as_ref());
    Ok(UpstreamRequest {
        method: spec.method.clone(),
        url,
        headers,
        body,
    })
}

pub(super) fn upstream_response(
    spec: &ResponseSpec,
    request: Option<UpstreamRequest>,
) -> Result<UpstreamResponse, Box<HelperError>> {
    let status = status_code(spec.status)?;
    let headers = header_map(&spec.headers)?;
    let body = body_state(spec.body.as_ref());
    Ok(UpstreamResponse {
        status,
        headers,
        body,
        request,
        captured: spec.captured.clone(),
    })
}

pub(super) fn status_code(status: u16) -> Result<StatusCode, Box<HelperError>> {
    StatusCode::from_u16(status)
        .map_err(|error| Box::new(failure("InvalidStatus", error.to_string())))
}

pub(super) fn header_map(pairs: &[Pair]) -> Result<HeaderMap, Box<HelperError>> {
    let mut headers = HeaderMap::new();
    for (name, value) in pairs {
        let header_name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|error| Box::new(failure("InvalidHeader", error.to_string())))?;
        let header_value = HeaderValue::from_str(value)
            .map_err(|error| Box::new(failure("InvalidHeader", error.to_string())))?;
        headers.append(header_name, header_value);
    }
    Ok(headers)
}

pub(super) fn redactor() -> Result<Redactor, Box<HelperError>> {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
        .map_err(|error| Box::new(failure("RedactionError", error.to_string())))
}

pub(super) fn runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| Box::new(failure("RuntimeError", error.to_string())))
}

pub(super) fn auth_resolution(name: &str) -> Result<AuthResolution, Box<HelperError>> {
    match name {
        "no-auth" => Ok(AuthResolution::NoAuth),
        "stored-user-token" => Ok(AuthResolution::StoredUserToken),
        "static-token" => Ok(AuthResolution::StaticToken),
        "per-request-header" => Ok(AuthResolution::PerRequestHeader),
        "oauth2-passthrough" => Ok(AuthResolution::Oauth2Passthrough),
        "m2m-client-credentials" => Ok(AuthResolution::ClientCredentials),
        "token-exchange" => Ok(AuthResolution::TokenExchange),
        "id-jag" => Ok(AuthResolution::IdJag),
        "aws-sigv4" => Ok(AuthResolution::AwsSigv4),
        "extra-headers" => Ok(AuthResolution::ExtraHeaders),
        "not-applicable" => Ok(AuthResolution::NotApplicable),
        "unresolved" => Ok(AuthResolution::Unresolved),
        "resolution-failed" => Ok(AuthResolution::Failed),
        "multiple" => Ok(AuthResolution::Multiple),
        other => Err(Box::new(failure(
            "ValueError",
            format!("unknown auth resolution: {other}"),
        ))),
    }
}

pub(super) fn failure(class: &str, message: String) -> HelperError {
    HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    }
}

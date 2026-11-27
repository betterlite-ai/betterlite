use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, header};
use betterlite_auth::trusted_network::decode::{DecodeError, TypedValue, decode_network_headers};
use betterlite_auth::trusted_network::proxy_utils::proxy_config_from_ranges;
use betterlite_auth::trusted_network::request::{NetworkRequest, resolve_network_context};
use betterlite_auth::trusted_network::settings::TrustedNetworkSettings;
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};

use crate::auth::auth_identity::{AuthContext, StageFailure, StageInput};
use crate::error_body::AuthenticationFailure;
use crate::state::ServerState;

mod tests;

const X_FORWARDED_FOR: &str = "x-forwarded-for";
const X_FORWARDED_PROTO: &str = "x-forwarded-proto";

#[derive(Clone, Debug)]
pub struct TrustedNetworkStageInput {
    pub context: AuthContext,
    pub peer: Option<String>,
    pub scheme: String,
    pub document: TypedValue,
}

// The request headers and the configured ranges become the outside byte document this stage decodes.
pub(crate) fn stage_input(
    request: &Request<Body>,
    authenticated: Option<&AuthContext>,
) -> Result<StageInput, StageFailure> {
    let context = authenticated
        .cloned()
        .ok_or(AuthenticationFailure::Missing)?;
    let slot = request
        .extensions()
        .get::<Arc<ServerState>>()
        .map(|state| state.trusted_network.clone());
    // general_settings falls back to {} when the proxy has not loaded it, which is every default
    let fallback = TrustedNetworkSettings::default();
    let settings = slot
        .as_ref()
        .map_or(&fallback, |entry| entry.settings.as_ref());
    let document = TypedValue {
        x_forwarded_for: header_text(request, X_FORWARDED_FOR),
        x_forwarded_proto: header_text(request, X_FORWARDED_PROTO),
        host: header_text(request, header::HOST.as_str()),
        trusted_proxy_ranges: settings.trusted_proxy_ranges.clone(),
        mcp_trusted_proxy_ranges: settings.mcp_trusted_proxy_ranges.clone(),
    };
    Ok(StageInput::TrustedNetwork(Box::new(
        TrustedNetworkStageInput {
            context,
            peer: request
                .extensions()
                .get::<ConnectInfo<SocketAddr>>()
                .map(|connect_info| connect_info.0.ip().to_string()),
            scheme: request.uri().scheme_str().unwrap_or_default().to_owned(),
            document,
        },
    )))
}

pub(crate) fn run_stage(stage: &TrustedNetworkStageInput) -> Result<AuthContext, StageFailure> {
    let bytes = serde_json::to_vec(&stage.document)
        .map_err(|error| decode_failure(&DecodeError::Write(error)))
        .map_err(|error| StageFailure::TrustedNetwork(Box::new(error)))?;
    let headers = decode_network_headers(&bytes)
        .map_err(|error| decode_failure(&error))
        .map_err(|error| StageFailure::TrustedNetwork(Box::new(error)))?;
    let config = proxy_config_from_ranges(headers.trusted_proxy_ranges.as_ref());
    let request = NetworkRequest {
        peer: stage.peer.as_deref(),
        x_forwarded_for: headers.x_forwarded_for.as_deref(),
        x_forwarded_proto: headers.x_forwarded_proto.as_deref(),
        host: headers.host.as_deref(),
        scheme: &stage.scheme,
    };
    let mut context = stage.context.clone();
    context.principal.network = resolve_network_context(&request, &config);
    Ok(context)
}

// LiteLLM decodes header bytes as latin-1, so every byte keeps the same char code
fn header_text(request: &Request<Body>, name: &str) -> Option<String> {
    request.headers().get(name).map(|value| {
        value
            .as_bytes()
            .iter()
            .map(|byte| char::from(*byte))
            .collect()
    })
}

// auth_exception_handler._as_proxy_exception: an unhandled exception inside the auth path is this public 401
fn decode_failure(error: &DecodeError) -> ProxyException {
    ProxyException::new(ProxyExceptionInit {
        message: format!("Authentication Error, {error}"),
        error_type: "auth_error".to_owned(),
        param: Some("None".to_owned()),
        code: ExceptionCode::Status(401_i64),
        headers: BTreeMap::new(),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    })
}

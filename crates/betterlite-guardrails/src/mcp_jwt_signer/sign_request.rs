use std::collections::BTreeMap;
use std::fmt::{self, Debug, Formatter};

use betterlite_prompts::arguments::json::Json;
use reqwest::StatusCode;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use thiserror::Error;

use crate::mcp_guardrail_translation::mapping_registration::McpCallType;
use crate::mcp_jwt_signer::JwtSigner;
use crate::mcp_jwt_signer::idp_documents::IdpError;
use crate::mcp_jwt_signer::incoming_token::{IncomingTokenError, verify_incoming_jwt};
use crate::mcp_jwt_signer::signing_key::SigningKeyError;
use crate::mcp_token_claims::build_channel_token_claims::build as build_channel_token_claims;
use crate::mcp_token_claims::build_claims::{ClaimRequest, build as build_claims};
use crate::mcp_token_claims::build_debug_header::render as build_debug_header;
use crate::mcp_token_claims::validate_required_claims::validate as validate_required_claims;
use crate::mcp_token_claims::{AuthFields, ClaimError, Claims};

mod tests;

const AUTHORIZATION_HEADER: &str = "Authorization";
const CHANNEL_TOKEN_HEADER: &str = "x-mcp-channel-token";
const DEBUG_HEADER: &str = "x-litellm-mcp-debug";

#[derive(Clone)]
pub struct HookData {
    pub tool_name: Option<String>,
    pub incoming_bearer_token: Option<SecretString>,
    pub extra_headers: Option<BTreeMap<String, String>>,
}

impl Debug for HookData {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("HookData")
            .field("tool_name", &self.tool_name)
            .field(
                "incoming_bearer_token",
                &self.incoming_bearer_token.as_ref().map(|_| "[redacted]"),
            )
            .field(
                "extra_headers",
                &self
                    .extra_headers
                    .as_ref()
                    .map(|headers| headers.keys().collect::<Vec<_>>()),
            )
            .finish()
    }
}

pub struct PreCallInput<'input> {
    pub auth: &'input AuthFields,
    pub jwt_claims: Option<&'input Claims>,
    pub data: &'input HookData,
    pub call_type: Option<McpCallType>,
    pub now: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ErrorDetail {
    pub error: String,
}

#[derive(Debug, Error)]
pub enum SignError {
    #[error("MCPJWTSigner: incoming token verification failed: {source}")]
    Verification {
        #[source]
        source: IncomingTokenError,
    },
    #[error("MCPJWTSigner: incoming token verification failed: {source}")]
    Introspection {
        #[source]
        source: IdpError,
    },
    #[error(transparent)]
    Claims(#[from] ClaimError),
    #[error("MCPJWTSigner: cannot sign the outbound token: {source}")]
    Signing {
        #[from]
        source: SigningKeyError,
    },
}

impl SignError {
    #[must_use]
    pub const fn status_code(&self) -> StatusCode {
        match self {
            Self::Verification { .. } | Self::Introspection { .. } => StatusCode::UNAUTHORIZED,
            Self::Claims(source) => source.status_code(),
            Self::Signing { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    #[must_use]
    pub fn detail(&self) -> ErrorDetail {
        ErrorDetail {
            error: self.to_string(),
        }
    }
}

pub async fn sign(signer: &JwtSigner, input: &PreCallInput<'_>) -> Result<HookData, SignError> {
    let Some(call_type) = input.call_type else {
        return Ok(input.data.clone());
    };
    if call_type == McpCallType::ListMcpTools && input.data.extra_headers.is_none() {
        return Ok(input.data.clone());
    }
    let tool_name = match call_type {
        McpCallType::ListMcpTools => None,
        McpCallType::CallMcpTool => input.data.tool_name.as_deref(),
    };
    let verified = verify_incoming_token(signer, input.data, input.now).await?;
    let incoming = verified.or_else(|| input.jwt_claims.cloned());
    let claim_request = ClaimRequest {
        auth: input.auth,
        tool_name,
        incoming_claims: incoming.as_ref(),
        call_type: Some(call_type),
    };
    let claims = outgoing_claims(signer, &claim_request, input.now)?;
    let headers = signed_headers(
        signer,
        &claims,
        input.now,
        input.data.extra_headers.as_ref(),
    )?;
    log_signed(signer, &claims, tool_name, incoming.is_some(), call_type);
    Ok(HookData {
        tool_name: tool_name.map(str::to_owned),
        incoming_bearer_token: input.data.incoming_bearer_token.clone(),
        extra_headers: Some(headers),
    })
}

fn log_signed(
    signer: &JwtSigner,
    claims: &Claims,
    tool_name: Option<&str>,
    verified: bool,
    call_type: McpCallType,
) {
    tracing::debug!(
        sub = %claims.get("sub").map_or_else(String::new, claim_text),
        act = %act_subject(claims),
        tool = ?tool_name,
        exp = %claims.get("exp").map_or_else(String::new, claim_text),
        verified,
        channel = signer.settings.channel_token_audience.is_some(),
        call_type = call_type_name(call_type),
        "MCPJWTSigner: signed JWT"
    );
}

fn outgoing_claims(
    signer: &JwtSigner,
    request: &ClaimRequest<'_>,
    now: i64,
) -> Result<Claims, SignError> {
    validate_required_claims(&signer.settings.required_claims, request.incoming_claims)?;
    let claims = build_claims(&signer.settings.claim_settings(), request, now)?;
    Ok(claims)
}

fn signed_headers(
    signer: &JwtSigner,
    claims: &Claims,
    now: i64,
    existing: Option<&BTreeMap<String, String>>,
) -> Result<BTreeMap<String, String>, SignError> {
    let mut headers = existing.cloned().unwrap_or_default();
    let token = signer.key.sign(claims)?;
    headers.insert(
        AUTHORIZATION_HEADER.to_owned(),
        format!("Bearer {}", token.expose_secret()),
    );
    if let Some(audience) = signer.settings.channel_token_audience.as_deref() {
        let channel_claims =
            build_channel_token_claims(claims, audience, signer.settings.channel_token_ttl, now)?;
        let channel_token = signer.key.sign(&channel_claims)?;
        headers.insert(
            CHANNEL_TOKEN_HEADER.to_owned(),
            format!("Bearer {}", channel_token.expose_secret()),
        );
    }
    if signer.settings.debug_headers {
        headers.insert(
            DEBUG_HEADER.to_owned(),
            build_debug_header(claims, &signer.key.kid),
        );
    }
    Ok(headers)
}

async fn verify_incoming_token(
    signer: &JwtSigner,
    data: &HookData,
    now: i64,
) -> Result<Option<Claims>, SignError> {
    if signer.settings.access_token_discovery_uri.is_none() {
        return Ok(None);
    }
    let Some(token) = data
        .incoming_bearer_token
        .as_ref()
        .filter(|token| !token.expose_secret().is_empty())
    else {
        tracing::debug!(
            "MCPJWTSigner: access_token_discovery_uri is configured but the request has no Bearer token; skipping verification"
        );
        return Ok(None);
    };
    if token.expose_secret().matches('.').count() == 2 {
        return verify_jwt(signer, token, now).await.map(Some);
    }
    let Some(endpoint) = signer.settings.token_introspection_endpoint.as_deref() else {
        tracing::warn!(
            "MCPJWTSigner: access_token_discovery_uri is set but the incoming token looks opaque and no token_introspection_endpoint is configured; proceeding without verification"
        );
        return Ok(None);
    };
    introspect(signer, endpoint, token).await.map(Some)
}

async fn verify_jwt(
    signer: &JwtSigner,
    token: &SecretString,
    now: i64,
) -> Result<Claims, SignError> {
    verify_incoming_jwt(signer, token, now)
        .await
        .map_err(verification_failed)
}

async fn introspect(
    signer: &JwtSigner,
    endpoint: &str,
    token: &SecretString,
) -> Result<Claims, SignError> {
    signer
        .idp
        .introspect(Some(endpoint), token)
        .await
        .map_err(introspection_failed)
}

fn verification_failed(source: IncomingTokenError) -> SignError {
    tracing::error!(
        error = %source,
        "MCPJWTSigner: incoming token verification failed"
    );
    SignError::Verification { source }
}

fn introspection_failed(source: IdpError) -> SignError {
    tracing::error!(
        error = %source,
        "MCPJWTSigner: incoming token verification failed"
    );
    SignError::Introspection { source }
}

fn claim_text(value: &Json) -> String {
    match value {
        Json::Text(text) => text.clone(),
        Json::Number(number) => number.to_string(),
        Json::Bool(flag) => flag.to_string(),
        Json::Null | Json::List(_) | Json::Object(_) => String::new(),
    }
}

fn act_subject(claims: &Claims) -> String {
    claims
        .get("act")
        .and_then(Json::as_object)
        .and_then(|act| act.get("sub"))
        .map_or_else(String::new, claim_text)
}

const fn call_type_name(call_type: McpCallType) -> &'static str {
    match call_type {
        McpCallType::CallMcpTool => "call_mcp_tool",
        McpCallType::ListMcpTools => "list_mcp_tools",
    }
}

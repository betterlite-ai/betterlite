use std::collections::BTreeMap;

use secrecy::SecretString;

use crate::mcp_guardrail_translation::mapping_registration::McpCallType;
use crate::mcp_jwt_signer::JwtSigner;
use crate::mcp_jwt_signer::sign_request::{HookData, PreCallInput, SignError, sign};
use crate::mcp_token_claims::{AuthFields, Claims};

mod tests;

const BEARER_PREFIX: &str = "bearer ";
const AUTHORIZATION_HEADER: &str = "authorization";

pub struct UpstreamRequest<'input> {
    pub auth: Option<&'input AuthFields>,
    pub jwt_claims: Option<&'input Claims>,
    pub extra_headers: Option<&'input BTreeMap<String, String>>,
    pub raw_headers: Option<&'input BTreeMap<String, String>>,
    pub for_list_tools: bool,
    pub tool_name: &'input str,
    pub now: i64,
}

pub async fn inject_upstream_headers(
    signer: &JwtSigner,
    request: &UpstreamRequest<'_>,
) -> Result<BTreeMap<String, String>, SignError> {
    let mut merged = request.extra_headers.cloned().unwrap_or_default();
    let Some(auth) = request.auth else {
        return Ok(merged);
    };
    let normalized: BTreeMap<String, String> = request
        .raw_headers
        .iter()
        .flat_map(|headers| headers.iter())
        .map(|(name, value)| (name.to_lowercase(), value.clone()))
        .collect();
    let incoming_bearer_token = normalized
        .get(AUTHORIZATION_HEADER)
        .filter(|value| value.to_lowercase().starts_with(BEARER_PREFIX))
        .and_then(|value| value.get(BEARER_PREFIX.len()..))
        .filter(|token| !token.is_empty())
        .map(|token| SecretString::from(token.to_owned()));
    let data = HookData {
        tool_name: (!request.for_list_tools).then(|| request.tool_name.to_owned()),
        incoming_bearer_token,
        extra_headers: Some(merged.clone()),
    };
    let call_type = if request.for_list_tools {
        McpCallType::ListMcpTools
    } else {
        McpCallType::CallMcpTool
    };
    let input = PreCallInput {
        auth,
        jwt_claims: request.jwt_claims,
        data: &data,
        call_type: Some(call_type),
        now: request.now,
    };
    let result = sign(signer, &input).await?;
    if let Some(headers) = result.extra_headers {
        merged.extend(headers);
    }
    Ok(merged)
}

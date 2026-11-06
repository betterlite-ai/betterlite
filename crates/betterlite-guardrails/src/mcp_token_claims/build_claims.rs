use betterlite_prompts::arguments::json::{Json, Object};
use serde_json::Number;

use crate::mcp_guardrail_translation::mapping_registration::McpCallType;
use crate::mcp_token_claims::apply_claim_operations::apply as apply_claim_operations;
use crate::mcp_token_claims::build_scope::generate as build_scope;
use crate::mcp_token_claims::passthrough_optional_claims::apply as passthrough_optional_claims;
use crate::mcp_token_claims::resolve_end_user_identity::resolve as resolve_end_user_identity;
use crate::mcp_token_claims::{AuthFields, ClaimError, ClaimSettings, Claims};

mod tests;

const PROXY_ACTOR: &str = "litellm-proxy";

#[derive(Clone, Copy, Debug)]
pub struct ClaimRequest<'input> {
    pub auth: &'input AuthFields,
    pub tool_name: Option<&'input str>,
    pub incoming_claims: Option<&'input Claims>,
    pub call_type: Option<McpCallType>,
}

pub fn build(
    settings: &ClaimSettings,
    request: &ClaimRequest<'_>,
    now: i64,
) -> Result<Claims, ClaimError> {
    let mut claims = timestamp_claims(settings, now)?;
    let subject = resolve_end_user_identity(
        &settings.end_user_claim_sources,
        request.auth,
        request.incoming_claims,
    );
    claims.insert("sub".to_owned(), Json::Text(subject));
    add_identity_claims(&mut claims, request.auth);
    let scope = build_scope(
        settings.allowed_scopes.as_deref(),
        request.tool_name,
        request.call_type,
    );
    claims.insert("scope".to_owned(), Json::Text(scope));
    passthrough_optional_claims(
        &mut claims,
        &settings.optional_claims,
        request.incoming_claims,
    );
    apply_claim_operations(
        &mut claims,
        &settings.add_claims,
        &settings.set_claims,
        &settings.remove_claims,
    );
    Ok(claims)
}

fn timestamp_claims(settings: &ClaimSettings, now: i64) -> Result<Claims, ClaimError> {
    let expires = now
        .checked_add(settings.ttl_seconds)
        .ok_or(ClaimError::TimestampOverflow)?;
    let mut claims = Claims::new();
    claims.insert("iss".to_owned(), Json::Text(settings.issuer.clone()));
    claims.insert("aud".to_owned(), Json::Text(settings.audience.clone()));
    claims.insert("iat".to_owned(), Json::Number(Number::from(now)));
    claims.insert("exp".to_owned(), Json::Number(Number::from(expires)));
    claims.insert("nbf".to_owned(), Json::Number(Number::from(now)));
    Ok(claims)
}

fn add_identity_claims(claims: &mut Claims, auth: &AuthFields) {
    if let Some(email) = auth.user_email.as_deref().filter(|value| !value.is_empty()) {
        claims.insert("email".to_owned(), Json::Text(email.to_owned()));
    }
    let actor = auth
        .team_id
        .as_deref()
        .filter(|value| !value.is_empty())
        .or_else(|| auth.org_id.as_deref().filter(|value| !value.is_empty()))
        .unwrap_or(PROXY_ACTOR);
    let mut act = Object::new();
    act.insert("sub".to_owned(), Json::Text(actor.to_owned()));
    claims.insert("act".to_owned(), Json::Object(act));
    if let Some(end_user_id) = auth
        .end_user_id
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        claims.insert("end_user_id".to_owned(), Json::Text(end_user_id.to_owned()));
    }
}

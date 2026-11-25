#![cfg(test)]

use std::collections::BTreeSet;

use betterlite_error::proxy_enums::special_headers::SpecialHeaders;
use http::StatusCode;

use super::error::{ANTHROPIC_MISSING_CREDENTIAL, CallerHeaderError, VERTEX_MISSING_CREDENTIAL};
use super::header_sets::{
    ANTHROPIC_CREDENTIAL_HEADERS, ANTHROPIC_NEVER_FORWARDED, BEDROCK_NEVER_FORWARDED,
    LITELLM_INTERNAL_CLAIMS, MAPPED_ROUTE_CALLER_KEY_HEADER, VERTEX_CREDENTIAL_HEADERS,
    VERTEX_NEVER_FORWARDED,
};

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

// B8410 owns the credential names; a set here is LiteLLM's base names plus the credentials the provider does not read
fn derived(base: &[&str], upstream: &[&str]) -> BTreeSet<String> {
    let mut names = set(base);
    names.extend(
        SpecialHeaders::litellm_credential_header_names()
            .into_iter()
            .filter(|name| !upstream.contains(&name.as_str())),
    );
    names
}

#[test]
fn the_upstream_credential_sets_match_litellm() {
    assert_eq!(
        VERTEX_CREDENTIAL_HEADERS,
        ["authorization", "x-goog-api-key"]
    );
    assert_eq!(ANTHROPIC_CREDENTIAL_HEADERS, ["authorization", "x-api-key"]);
}

#[test]
fn the_mapped_route_header_matches_litellm() {
    assert_eq!(MAPPED_ROUTE_CALLER_KEY_HEADER, "litellm_user_api_key");
}

#[test]
fn the_internal_claim_list_matches_litellm() {
    assert_eq!(
        LITELLM_INTERNAL_CLAIMS,
        [
            "_litellm_jwt_issuer",
            "_litellm_user_id",
            "_litellm_user_email",
            "_litellm_team_id",
            "_litellm_team_ids",
            "_litellm_org_id",
            "_litellm_end_user_id",
        ]
    );
}

#[test]
fn every_never_forwarded_set_matches_litellms_literal_names() {
    assert_eq!(
        set(&VERTEX_NEVER_FORWARDED),
        set(&[
            "api-key",
            "content-length",
            "host",
            "ocp-apim-subscription-key",
            "x-api-key",
            "x-litellm-api-key",
        ])
    );
    assert_eq!(
        set(&ANTHROPIC_NEVER_FORWARDED),
        set(&[
            "accept-encoding",
            "api-key",
            "content-length",
            "host",
            "ocp-apim-subscription-key",
            "x-goog-api-key",
            "x-litellm-api-key",
        ])
    );
    assert_eq!(
        set(&BEDROCK_NEVER_FORWARDED),
        set(&[
            "accept-encoding",
            "api-key",
            "authorization",
            "content-length",
            "host",
            "ocp-apim-subscription-key",
            "x-api-key",
            "x-goog-api-key",
            "x-litellm-api-key",
        ])
    );
}

#[test]
fn the_vertex_set_is_the_base_names_plus_the_unused_credentials() {
    assert_eq!(
        set(&VERTEX_NEVER_FORWARDED),
        derived(&["content-length", "host"], &VERTEX_CREDENTIAL_HEADERS)
    );
}

#[test]
fn the_anthropic_set_is_the_base_names_plus_the_unused_credentials() {
    assert_eq!(
        set(&ANTHROPIC_NEVER_FORWARDED),
        derived(
            &["accept-encoding", "content-length", "host"],
            &ANTHROPIC_CREDENTIAL_HEADERS
        )
    );
}

#[test]
fn the_bedrock_set_is_the_base_names_plus_every_credential() {
    assert_eq!(
        set(&BEDROCK_NEVER_FORWARDED),
        derived(&["accept-encoding", "content-length", "host"], &[])
    );
}

#[test]
fn the_vertex_error_keeps_litellms_message() {
    assert_eq!(
        VERTEX_MISSING_CREDENTIAL,
        "No Vertex AI credential is configured on this proxy and the request carried no upstream Google credential. The LiteLLM virtual key is not forwarded to Google. Configure a Vertex credential (DEFAULT_VERTEXAI_PROJECT / DEFAULT_VERTEXAI_LOCATION / DEFAULT_VERTEXAI_CREDENTIALS, or a model with use_in_pass_through: true), or send your own Google OAuth token in the Authorization header."
    );
    assert_eq!(
        CallerHeaderError::VertexCredentialMissing.to_string(),
        VERTEX_MISSING_CREDENTIAL
    );
}

#[test]
fn the_anthropic_error_keeps_litellms_message() {
    assert_eq!(
        ANTHROPIC_MISSING_CREDENTIAL,
        "No Anthropic credential is configured on this proxy and the request carried no upstream Anthropic credential. The LiteLLM virtual key is not forwarded to Anthropic. Configure an Anthropic credential (ANTHROPIC_API_KEY or ANTHROPIC_AUTH_TOKEN, or a model with use_in_pass_through: true), or send your own Anthropic API key in the x-api-key header or your own Anthropic OAuth token in the Authorization header."
    );
    assert_eq!(
        CallerHeaderError::AnthropicCredentialMissing.to_string(),
        ANTHROPIC_MISSING_CREDENTIAL
    );
}

#[test]
fn both_missing_credential_errors_are_401() {
    assert_eq!(
        CallerHeaderError::VertexCredentialMissing.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        CallerHeaderError::AnthropicCredentialMissing.status(),
        StatusCode::UNAUTHORIZED
    );
}

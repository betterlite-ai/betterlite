#![cfg(test)]

use crate::gateway_dcr::contracts::{
    ACCESS_TOKEN_TOKEN_TYPE, AUTH_CODE_DEBUG_KEY, CLAIM_TTL_BUFFER_SECONDS,
    CLIENT_RECORD_DEBUG_KEY, CODE_CLAIM_KEY_PREFIX, CONNECT_FLOW_COOKIE_PREFIX,
    CONNECT_FLOW_DEBUG_KEY, CONNECT_FLOW_TTL_SECONDS, DB_FAULTED_DESCRIPTION,
    DB_UNAVAILABLE_DESCRIPTION, FLOW_CLAIM_KEY_PREFIX, GATEWAY_AUTH_CODE_PREFIX,
    GATEWAY_AUTH_CODE_TTL_SECONDS, GATEWAY_DCR_CLIENT_ID_PREFIX,
    MANUAL_DELIVERY_AUTH_CODE_TTL_SECONDS, MAX_CLIENT_ID_LENGTH, MAX_CODE_VERIFIER_LENGTH,
    MAX_REDIRECT_URI_LENGTH, MAX_REDIRECT_URIS, MAX_STATE_LENGTH, MIN_CODE_VERIFIER_LENGTH,
    PROXY_API_AUDIENCE, REFRESH_CLAIM_KEY_PREFIX, SUBJECT_TOKEN_TYPES, TOKEN_EXCHANGE_GRANT_TYPE,
    supported_grant_types,
};
use crate::gateway_dcr::ports::{
    LookupServerReachability as _, LookupVendorCredential as _, MintProxyCredential as _,
    ProxyCredentialMintFailure, RefuseProxyCredential, RefuseSubjectToken,
    UnavailableVendorCredential, UnreachableServer, VendorCredentialState,
};
use crate::gateway_dcr::subject_token::{ExchangeSubjectToken as _, RefusalCode};
use crate::types::SessionAudience;

#[test]
fn prefixes_ttls_and_bounds_match_litellm() {
    assert_eq!(GATEWAY_DCR_CLIENT_ID_PREFIX, "llm_dcrc_");
    assert_eq!(GATEWAY_AUTH_CODE_PREFIX, "llm_gcode_");
    assert_eq!(CONNECT_FLOW_COOKIE_PREFIX, "mcp_connect_flow_");
    assert_eq!(CONNECT_FLOW_TTL_SECONDS, 600);
    assert_eq!(GATEWAY_AUTH_CODE_TTL_SECONDS, 120);
    assert_eq!(MANUAL_DELIVERY_AUTH_CODE_TTL_SECONDS, 300);
    assert_eq!(CLAIM_TTL_BUFFER_SECONDS, 60);
    assert_eq!(MAX_REDIRECT_URIS, 4);
    assert_eq!(MAX_REDIRECT_URI_LENGTH, 256);
    assert_eq!(MAX_CLIENT_ID_LENGTH, 2048);
    assert_eq!(MAX_STATE_LENGTH, 1024);
    assert_eq!(MIN_CODE_VERIFIER_LENGTH, 43);
    assert_eq!(MAX_CODE_VERIFIER_LENGTH, 128);
}

#[test]
fn claim_keys_and_debug_keys_match_litellm() {
    assert_eq!(CODE_CLAIM_KEY_PREFIX, "mcp_gateway_dcr_code_used:");
    assert_eq!(FLOW_CLAIM_KEY_PREFIX, "mcp_gateway_dcr_flow_used:");
    assert_eq!(REFRESH_CLAIM_KEY_PREFIX, "mcp_gateway_dcr_refresh_used:");
    assert_eq!(CLIENT_RECORD_DEBUG_KEY, "gateway_dcr_client");
    assert_eq!(CONNECT_FLOW_DEBUG_KEY, "gateway_connect_flow");
    assert_eq!(AUTH_CODE_DEBUG_KEY, "gateway_authorization_code");
}

#[test]
fn database_failure_descriptions_match_litellm() {
    assert_eq!(
        DB_UNAVAILABLE_DESCRIPTION,
        "the gateway database is unavailable; retry"
    );
    assert_eq!(
        DB_FAULTED_DESCRIPTION,
        "the gateway database reported a fault that is not a transient outage; retrying will not \
         help until the gateway deployment is repaired"
    );
}

#[test]
fn proxy_api_audience_is_the_proxy_api_session_audience() {
    assert_eq!(PROXY_API_AUDIENCE, SessionAudience::ProxyApi);
}

#[test]
fn supported_grant_types_lists_the_two_base_grants_without_token_exchange() {
    assert_eq!(
        supported_grant_types(false),
        vec!["authorization_code", "refresh_token"]
    );
}

#[test]
fn supported_grant_types_appends_token_exchange_only_when_available() {
    assert_eq!(
        supported_grant_types(true),
        vec![
            "authorization_code",
            "refresh_token",
            TOKEN_EXCHANGE_GRANT_TYPE
        ]
    );
    assert_eq!(
        TOKEN_EXCHANGE_GRANT_TYPE,
        "urn:ietf:params:oauth:grant-type:token-exchange"
    );
    assert!(!supported_grant_types(false).contains(&TOKEN_EXCHANGE_GRANT_TYPE));
}

#[test]
fn subject_token_types_cover_the_token_exchange_family() {
    assert_eq!(
        ACCESS_TOKEN_TOKEN_TYPE,
        "urn:ietf:params:oauth:token-type:access_token"
    );
    assert_eq!(SUBJECT_TOKEN_TYPES.len(), 3);
    assert!(SUBJECT_TOKEN_TYPES.contains(&"urn:ietf:params:oauth:token-type:jwt"));
    assert!(SUBJECT_TOKEN_TYPES.contains(&"urn:ietf:params:oauth:token-type:id_token"));
    assert!(SUBJECT_TOKEN_TYPES.contains(&ACCESS_TOKEN_TOKEN_TYPE));
}

#[tokio::test]
async fn refuse_proxy_credential_answers_unresolvable() {
    let failure = RefuseProxyCredential
        .mint("user-1", None)
        .await
        .unwrap_err();
    assert_eq!(failure, ProxyCredentialMintFailure::Unresolvable);
}

#[tokio::test]
async fn refuse_subject_token_answers_unsupported_grant_type() {
    let refusal = RefuseSubjectToken
        .exchange("subject", &())
        .await
        .unwrap_err();
    assert_eq!(refusal.error, RefusalCode::UnsupportedGrantType);
    assert_eq!(
        refusal.description,
        "this gateway is not configured to exchange IdP tokens"
    );
}

#[tokio::test]
async fn unavailable_vendor_credential_answers_unavailable() {
    assert_eq!(
        UnavailableVendorCredential.lookup("user-1", "srv-1").await,
        VendorCredentialState::Unavailable
    );
}

#[tokio::test]
async fn unreachable_server_answers_false() {
    assert!(!UnreachableServer.reachable("user-1", "srv-1").await);
}

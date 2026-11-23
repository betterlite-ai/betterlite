#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};

use betterlite_db::oauth_token_resolution::{EnforceRequest, OauthBindingPort};
use http::StatusCode;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::{BindingDenied, RefreshOwnership, RejectionCode};

fn secret(value: &str) -> SecretString {
    SecretString::from(value)
}

struct FakeBindingPort;

impl OauthBindingPort for FakeBindingPort {
    type Server = ();
    type Ownership = RefreshOwnership;
    type Denied = BindingDenied;

    fn enforce(
        &self,
        request: &EnforceRequest<'_, Self::Server, Self::Ownership>,
    ) -> impl Future<Output = Result<Option<String>, Self::Denied>> + Send {
        match request.ownership {
            Some(RefreshOwnership::Presented(_)) => ready(Err(BindingDenied {
                code: RejectionCode::PrincipalMismatch,
                description: "the presented refresh token does not match the stored one".to_owned(),
                server_id: "srv-1".to_owned(),
            })),
            Some(RefreshOwnership::Proven) | None => ready(Ok(None)),
        }
    }

    fn proof_matches(
        &self,
        _server: &Self::Server,
        _user_id: &str,
        _stored_proof: Option<&str>,
    ) -> impl Future<Output = bool> + Send {
        ready(true)
    }
}

#[rstest]
#[case::principal_mismatch(RejectionCode::PrincipalMismatch, "oauth_principal_mismatch")]
#[case::binding_failed(RejectionCode::BindingFailed, "oauth_identity_binding_failed")]
fn rejection_code_literals(#[case] code: RejectionCode, #[case] literal: &str) {
    assert_eq!(code.as_str(), literal);
    assert_eq!(
        serde_json::to_value(code).unwrap(),
        serde_json::json!(literal)
    );
}

#[rstest]
#[case::principal_mismatch(RejectionCode::PrincipalMismatch)]
#[case::binding_failed(RejectionCode::BindingFailed)]
fn denial_status_and_detail(#[case] code: RejectionCode) {
    let description = "The browser account \"Zoë\" does not match the selected credential owner.";
    let server_id = "srv-\"bü\"";
    let denial = BindingDenied {
        code,
        description: description.to_owned(),
        server_id: server_id.to_owned(),
    };
    assert_eq!(denial.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        serde_json::to_value(denial.detail()).unwrap(),
        serde_json::json!({
            "error": code.as_str(),
            "error_description": description,
            "server_id": server_id,
            "credential_owner": "caller",
            "credential_stored": false,
        })
    );
}

#[test]
fn ownership_from_secret() {
    let RefreshOwnership::Presented(carried) = RefreshOwnership::from(secret("rt-secret-1")) else {
        panic!("From<SecretString> must build a Presented ownership");
    };
    assert_eq!(carried.expose_secret(), "rt-secret-1");
}

#[test]
fn ownership_debug_redacts() {
    let ownership = RefreshOwnership::from(secret("rt-secret-1"));
    let printed = format!("{ownership:?}");
    assert!(printed.contains("Presented"));
    assert!(!printed.contains("rt-secret-1"));
}

#[tokio::test]
async fn port_associated_types() {
    let port = FakeBindingPort;
    let token_response = BTreeMap::new();
    let presented = RefreshOwnership::from(secret("rt-secret-1"));
    let denied = port
        .enforce(&EnforceRequest {
            server: &(),
            token_response: &token_response,
            user_id: Some("user-1"),
            grant_type: "refresh_token",
            ownership: Some(&presented),
        })
        .await
        .unwrap_err();
    assert_eq!(denied.code, RejectionCode::PrincipalMismatch);
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);

    let proven = RefreshOwnership::Proven;
    let allowed = port
        .enforce(&EnforceRequest {
            server: &(),
            token_response: &token_response,
            user_id: Some("user-1"),
            grant_type: "refresh_token",
            ownership: Some(&proven),
        })
        .await;
    assert_eq!(allowed, Ok(None));

    let anonymous = port
        .enforce(&EnforceRequest {
            server: &(),
            token_response: &token_response,
            user_id: None,
            grant_type: "refresh_token",
            ownership: None,
        })
        .await;
    assert_eq!(anonymous, Ok(None));
}

use std::sync::atomic::{AtomicUsize, Ordering};

use secrecy::ExposeSecret as _;
use tokio::sync::OnceCell;

use super::fixture::{Refused, Scripted, access};
use crate::proxy_auth::error::AzureCredentialError;
use crate::proxy_auth::{AzureADCredential, TokenCredential as _};

static DEFAULT_CHAINS_BUILT: AtomicUsize = AtomicUsize::new(0);

fn build_default_chain() -> Scripted {
    DEFAULT_CHAINS_BUILT.fetch_add(1, Ordering::SeqCst);
    Scripted::answering(vec![
        Ok(access("from-default", 5_000)),
        Ok(access("from-default-again", 6_000)),
    ])
}

#[tokio::test]
async fn an_injected_credential_is_used_and_no_default_chain_is_needed() {
    let credential = AzureADCredential {
        credential: OnceCell::new_with(Some(Scripted::answering(vec![Ok(access(
            "injected", 9_000,
        ))]))),
        default_chain: None,
    };
    let token = credential.get_token("api://x/.default").await.unwrap();
    assert_eq!(token.token.expose_secret(), "injected");
    assert_eq!(token.expires_on, 9_000);
    let inner = credential.credential.get().unwrap();
    assert_eq!(*inner.scopes.lock().unwrap(), ["api://x/.default"]);
}

#[tokio::test]
async fn the_default_chain_is_built_on_the_first_call_and_kept() {
    let credential = AzureADCredential {
        credential: OnceCell::new(),
        default_chain: Some(build_default_chain),
    };
    assert_eq!(DEFAULT_CHAINS_BUILT.load(Ordering::SeqCst), 0);
    assert!(credential.credential.get().is_none());
    let first = credential.get_token("scope").await.unwrap();
    let second = credential.get_token("scope").await.unwrap();
    assert_eq!(first.token.expose_secret(), "from-default");
    assert_eq!(second.token.expose_secret(), "from-default-again");
    assert_eq!(DEFAULT_CHAINS_BUILT.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_missing_azure_identity_fails_at_the_first_call_with_the_install_command() {
    let credential: AzureADCredential<Scripted> = AzureADCredential {
        credential: OnceCell::new(),
        default_chain: None,
    };
    let first = credential.get_token("scope").await.unwrap_err();
    let second = credential.get_token("scope").await.unwrap_err();
    assert!(matches!(first, AzureCredentialError::ImportError));
    assert_eq!(
        second.to_string(),
        "azure-identity is required for AzureADCredential. Install it with: pip install azure-identity"
    );
}

#[tokio::test]
async fn the_wrapped_credential_error_reaches_the_caller() {
    let credential = AzureADCredential {
        credential: OnceCell::new_with(Some(Scripted::answering(vec![Err(Refused(
            "no login".to_owned(),
        ))]))),
        default_chain: None,
    };
    let failure = credential.get_token("scope").await.unwrap_err();
    assert!(
        matches!(failure, AzureCredentialError::Credential(Refused(reason)) if reason == "no login")
    );
}

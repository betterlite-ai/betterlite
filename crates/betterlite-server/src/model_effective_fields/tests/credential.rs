use std::sync::Mutex;

use betterlite_config::credentials::{CredentialItem, CredentialValues};
use betterlite_db::identity_rows::Payload;
use betterlite_error::proxy_error_types::ProxyErrorTypes;
use rstest::rstest;

use crate::credentials::registry::CredentialRegistry;
use crate::model_effective_fields::credential::{
    CredentialSources, check_credential_name, stored_credential_name,
};
use crate::model_effective_fields::error::ModelFieldError;

use super::support::{Store, cipher, object, sealed};

fn registry(names: &[&str]) -> CredentialRegistry {
    let registry = CredentialRegistry::default();
    registry.upsert_credentials(
        names
            .iter()
            .map(|name| CredentialItem {
                credential_name: (*name).to_owned(),
                credential_info: Payload::default(),
                credential_values: CredentialValues::default(),
            })
            .collect(),
    );
    registry
}

async fn checked(
    incoming: &str,
    existing: &str,
    loaded: &[&str],
    store: &Store,
) -> Result<(), ModelFieldError> {
    let cipher = cipher();
    let registry = registry(loaded);
    let sources = CredentialSources {
        cipher: &cipher,
        registry: &registry,
        store,
    };
    check_credential_name(Some(&object(incoming)), Some(&object(existing)), &sources).await
}

#[rstest]
#[case::omitted("{}")]
#[case::null_detaches(r#"{"litellm_credential_name": null}"#)]
#[tokio::test]
async fn unsent_credential_names_are_not_looked_up(#[case] incoming: &str) {
    let store = Store::default();
    checked(incoming, "{}", &[], &store).await.unwrap();
    assert!(store.lookups.lock().unwrap().is_empty());
}

#[tokio::test]
async fn an_empty_credential_name_is_refused_before_any_lookup() {
    let store = Store::default();
    let error = checked(
        r#"{"litellm_credential_name": ""}"#,
        "{}",
        &["", "x"],
        &store,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, ModelFieldError::EmptyCredentialName));
    assert_eq!(error.status(), 400);
    assert_eq!(error.error_type(), Some(ProxyErrorTypes::ValidationError));
    assert_eq!(error.param().as_deref(), Some("litellm_credential_name"));
    assert!(store.lookups.lock().unwrap().is_empty());
}

#[tokio::test]
async fn the_stored_credential_is_kept_without_a_lookup() {
    let store = Store::default();
    let existing = format!(r#"{{"litellm_credential_name": "{}"}}"#, sealed("retired"));
    checked(
        r#"{"litellm_credential_name": "retired"}"#,
        &existing,
        &[],
        &store,
    )
    .await
    .unwrap();
    assert!(store.lookups.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_loaded_credential_skips_the_table() {
    let store = Store::default();
    checked(
        r#"{"litellm_credential_name": "azure"}"#,
        "{}",
        &["azure"],
        &store,
    )
    .await
    .unwrap();
    assert!(store.lookups.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_stored_credential_row_is_accepted() {
    let store = Store {
        names: vec!["azure".to_owned()],
        broken: false,
        lookups: Mutex::new(Vec::new()),
    };
    checked(
        r#"{"litellm_credential_name": "azure"}"#,
        "{}",
        &["other"],
        &store,
    )
    .await
    .unwrap();
    assert_eq!(*store.lookups.lock().unwrap(), ["azure"]);
}

#[tokio::test]
async fn an_unknown_credential_is_refused() {
    let store = Store {
        names: vec!["other".to_owned()],
        broken: false,
        lookups: Mutex::new(Vec::new()),
    };
    let existing = format!(r#"{{"litellm_credential_name": "{}"}}"#, sealed("old"));
    let error = checked(
        r#"{"litellm_credential_name": "azure"}"#,
        &existing,
        &[],
        &store,
    )
    .await
    .unwrap_err();
    match &error {
        ModelFieldError::CredentialNotFound { credential_name } => {
            assert_eq!(credential_name, "azure");
        }
        other => panic!("expected a missing credential: {other:?}"),
    }
    assert_eq!(error.status(), 400);
    assert_eq!(error.param().as_deref(), Some("litellm_credential_name"));
}

#[tokio::test]
async fn a_failing_credential_table_is_a_server_error() {
    let store = Store {
        names: Vec::new(),
        broken: true,
        lookups: Mutex::new(Vec::new()),
    };
    let error = checked(r#"{"litellm_credential_name": "azure"}"#, "{}", &[], &store)
        .await
        .unwrap_err();
    assert!(matches!(error, ModelFieldError::CredentialLookup { .. }));
    assert_eq!(error.status(), 500);
    assert_eq!(error.error_type(), None);
    assert_eq!(error.param(), None);
}

#[test]
fn stored_credential_name_opens_only_text() {
    let cipher = cipher();
    assert_eq!(stored_credential_name(&cipher, None), None);
    let null = object(r#"{"litellm_credential_name": null}"#);
    assert_eq!(stored_credential_name(&cipher, Some(&null)), None);
    let plain = object(r#"{"litellm_credential_name": "plain"}"#);
    assert_eq!(
        stored_credential_name(&cipher, Some(&plain)).as_deref(),
        Some("plain")
    );
}

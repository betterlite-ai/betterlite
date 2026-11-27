#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::credentials::{CredentialItem, CredentialValues};
use betterlite_core::rag::ingestion::credentials::{CredentialLookup as _, CredentialLookupError};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::Payload;

use super::CredentialLookup;
use crate::credentials::registry::CredentialRegistry;

fn values(api_key: &str) -> CredentialValues {
    let fields = BTreeMap::from([("api_key".to_owned(), JsonData::String(api_key.to_owned()))]);
    CredentialValues::from(Payload(fields))
}

fn registry(credential_name: &str, api_key: &str) -> Arc<CredentialRegistry> {
    let store = CredentialRegistry::default();
    store.upsert_credentials(vec![CredentialItem {
        credential_name: credential_name.to_owned(),
        credential_info: Payload(BTreeMap::new()),
        credential_values: values(api_key),
    }]);
    Arc::new(store)
}

#[test]
fn credential_lookup_returns_the_values_of_a_known_name() {
    let port = CredentialLookup {
        registry: registry("rag-store", "sk-live-secret"),
    };
    let found = port
        .credential_values("rag-store")
        .expect("the credential is found");
    assert_eq!(found, values("sk-live-secret"));
}

#[test]
fn credential_lookup_reports_an_unknown_name_as_not_found() {
    let port = CredentialLookup {
        registry: registry("rag-store", "sk-live-secret"),
    };
    let error = port
        .credential_values("missing")
        .expect_err("the credential is unknown");
    assert_eq!(
        error,
        CredentialLookupError::NotFound {
            credential_name: "missing".to_owned()
        }
    );
}

#[test]
fn credential_lookup_shows_no_value_in_debug_or_error_text() {
    let port = CredentialLookup {
        registry: registry("rag-store", "sk-live-secret"),
    };
    let debug = format!("{port:?}");
    assert_eq!(debug, "CredentialLookup { .. }");
    let found = port
        .credential_values("rag-store")
        .expect("the credential is found");
    assert!(!format!("{found:?}").contains("sk-live-secret"));
    let missing = port
        .credential_values("missing")
        .expect_err("the credential is unknown");
    let text = format!("{missing} {missing:?}");
    assert!(!text.contains("sk-live-secret"));
    assert!(text.contains("missing"));
}

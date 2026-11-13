#![cfg(test)]

use super::{load_google_kms, tests::Transport};
use crate::google_kms::{GOOGLE_KMS_CLIENT, GOOGLE_KMS_RESOURCE_NAME};
use std::{collections::BTreeMap, sync::Arc};

#[test]
fn client_storage_replaces_client_and_resource_name() {
    let mut environment = BTreeMap::from([
        (
            "GOOGLE_APPLICATION_CREDENTIALS".to_owned(),
            "credentials.json".to_owned(),
        ),
        ("GOOGLE_KMS_RESOURCE_NAME".to_owned(), "first".to_owned()),
    ]);
    load_google_kms(Some(true), &environment, Arc::new(Transport)).unwrap();
    environment.insert("GOOGLE_KMS_RESOURCE_NAME".to_owned(), "second".to_owned());
    load_google_kms(Some(true), &environment, Arc::new(Transport)).unwrap();
    assert_eq!(
        GOOGLE_KMS_CLIENT
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .resource_name,
        "second"
    );
    assert_eq!(
        GOOGLE_KMS_RESOURCE_NAME.lock().unwrap().as_deref(),
        Some("second")
    );
}

#![cfg(test)]

use super::{load_google_kms, tests::Transport};
use crate::google_kms::{DecryptTransport, GOOGLE_KMS_CLIENT};
use std::{collections::BTreeMap, path::Path, sync::Arc};

#[test]
fn load_google_kms_does_not_initialize_when_disabled() {
    load_google_kms(Some(false), &BTreeMap::new(), Arc::new(Transport)).unwrap();
    assert!(GOOGLE_KMS_CLIENT.lock().unwrap().is_none());
}

#[tokio::test]
async fn load_google_kms_keeps_credentials_resource_and_transport() {
    let environment = BTreeMap::from([
        (
            "GOOGLE_APPLICATION_CREDENTIALS".to_owned(),
            "credentials.json".to_owned(),
        ),
        ("GOOGLE_KMS_RESOURCE_NAME".to_owned(), "resource".to_owned()),
    ]);
    let transport: Arc<dyn DecryptTransport> = Arc::new(Transport);
    load_google_kms(Some(true), &environment, Arc::clone(&transport)).unwrap();
    let client = Arc::clone(GOOGLE_KMS_CLIENT.lock().unwrap().as_ref().unwrap());
    assert_eq!(client.credentials_path, Path::new("credentials.json"));
    assert_eq!(client.resource_name, "resource");
    assert!(Arc::ptr_eq(&client.transport, &transport));
    assert_eq!(
        client
            .transport
            .decrypt(
                &client.credentials_path,
                &client.resource_name,
                b"ciphertext"
            )
            .await
            .unwrap(),
        b"plaintext"
    );
}

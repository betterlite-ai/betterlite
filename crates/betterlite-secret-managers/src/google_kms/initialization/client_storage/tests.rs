#![cfg(test)]

use super::tests::Transport;
use crate::google_kms::Client;
use std::sync::Arc;

use super::store_client;
use crate::google_kms::{GOOGLE_KMS_CLIENT, GOOGLE_KMS_RESOURCE_NAME};

#[tokio::test]
async fn client_storage_keeps_client_and_resource_name_in_process_wide_cells() {
    let client = Client {
        resource_name: "projects/p/locations/l/keyRings/r/cryptoKeys/k".to_owned(),
        transport: Arc::new(Transport),
    };
    store_client(client).unwrap();
    assert!(GOOGLE_KMS_CLIENT.lock().unwrap().is_some());
    assert_eq!(
        GOOGLE_KMS_RESOURCE_NAME.lock().unwrap().as_deref(),
        Some("projects/p/locations/l/keyRings/r/cryptoKeys/k")
    );
}

#![cfg(test)]

use super::{build_client, store_client};
use crate::google_kms::{GOOGLE_KMS_CLIENT, GOOGLE_KMS_RESOURCE_NAME};

#[tokio::test]
async fn client_storage_keeps_client_and_resource_name_in_process_wide_cells() {
    let client = build_client().await.unwrap();
    store_client(
        client,
        "projects/p/locations/l/keyRings/r/cryptoKeys/k".to_owned(),
    )
    .unwrap();
    assert!(GOOGLE_KMS_CLIENT.lock().unwrap().is_some());
    assert_eq!(
        GOOGLE_KMS_RESOURCE_NAME.lock().unwrap().as_deref(),
        Some("projects/p/locations/l/keyRings/r/cryptoKeys/k")
    );
}

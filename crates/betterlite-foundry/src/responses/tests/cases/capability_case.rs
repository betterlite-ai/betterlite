use std::hint::black_box;

use betterlite_models::identity::provider_id::ProviderId;

use crate::responses::{PROVIDER, SUPPORTS_NATIVE_WEBSOCKET};

pub(in crate::responses::tests) fn no_native_websocket() {
    assert_eq!(PROVIDER, ProviderId::AzureAi);
    assert_eq!(PROVIDER.to_string(), "azure_ai");
    assert!(!black_box(SUPPORTS_NATIVE_WEBSOCKET));
}

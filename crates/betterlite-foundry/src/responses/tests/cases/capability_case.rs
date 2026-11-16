use betterlite_models::identity::provider_id::ProviderId;

use crate::responses::{PROVIDER, SUPPORTS_NATIVE_WEBSOCKET};

pub(super) fn no_native_websocket() {
    assert_eq!(PROVIDER, ProviderId::AzureAi);
    assert_eq!(PROVIDER.to_string(), "azure_ai");
    assert!(!SUPPORTS_NATIVE_WEBSOCKET);
}

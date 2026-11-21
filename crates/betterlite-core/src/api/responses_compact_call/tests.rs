#![cfg(test)]

mod compact_model_and_encrypted_items;
mod compact_native_support_required;
mod compact_optional_parameters;
mod compact_timeout_and_residency;
mod support;

#[tokio::test]
async fn compact_model_and_encrypted_items() {
    compact_model_and_encrypted_items::sync_and_async().await;
    compact_model_and_encrypted_items::routing_prefix().await;
}

#[tokio::test]
async fn compact_optional_parameters() {
    compact_optional_parameters::forwarded().await;
    compact_optional_parameters::unsupported_rejected().await;
    compact_optional_parameters::drop_params_normalized().await;
    compact_optional_parameters::global_drop_params().await;
    compact_optional_parameters::nested_drop_params().await;
    compact_optional_parameters::bypass_and_nulls().await;
}

#[tokio::test]
async fn compact_timeout_and_residency() {
    compact_timeout_and_residency::default_timeouts().await;
    compact_timeout_and_residency::configured_timeouts().await;
    compact_timeout_and_residency::residency().await;
    compact_timeout_and_residency::async_handoff().await;
    compact_timeout_and_residency::explicit_provider_skips_lookup().await;
    compact_timeout_and_residency::client_parameters().await;
    compact_timeout_and_residency::sync_markers().await;
    compact_timeout_and_residency::true_marker().await;
}

#[tokio::test]
async fn compact_native_support_required() {
    compact_native_support_required::native_config_required().await;
    compact_native_support_required::missing_provider().await;
    compact_native_support_required::provider_auth_error().await;
}

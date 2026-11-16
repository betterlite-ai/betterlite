#![expect(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "Helper operations take and return arbitrary provider JSON as serde_json::Value and build it with serde_json::json!."
)]

betterlite_oracle_helper_api::helper_group! {
    "b4742" => b4742, "b5082" => b5082, "b7325" => b7325,
    "openai_mock_transport" => openai_mock_transport, "proxy_policy" => proxy_policy,
    "skill_discovery_archives" => skill_discovery_archives, "tls_policy" => tls_policy,
    "url_destination" => url_destination, "url_origin" => url_origin, "url_path" => url_path,
}

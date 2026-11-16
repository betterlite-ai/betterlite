#![expect(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "Helper operations take and return arbitrary provider JSON as serde_json::Value and build it with serde_json::json!."
)]

betterlite_oracle_helper_api::helper_group! {
    "b4321" => b4321, "b4325" => b4325, "b4326" => b4326, "b4327" => b4327, "b4329" => b4329,
    "b4330" => b4330, "b4331" => b4331, "b5640" => b5640,
    "redis_cluster_node_isolation" => cluster_node_isolation,
}

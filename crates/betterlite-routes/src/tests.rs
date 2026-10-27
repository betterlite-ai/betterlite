#![cfg(test)]

use super::{BACKEND_PATH_PREFIXES, GATEWAY_PATH_PREFIXES};

#[test]
fn route_prefixes_preserve_dynamic_templates() {
    assert!(GATEWAY_PATH_PREFIXES.contains(&"/{provider}/"));
    assert!(BACKEND_PATH_PREFIXES.contains(&"/{mcp_server_name}/"));
}

#[test]
fn route_mounts_are_component_specific() {
    assert!(super::GATEWAY_MOUNT_PATHS.contains("/metrics"));
    assert!(super::BACKEND_MOUNT_PATHS.contains("/swagger"));
    assert!(super::BACKEND_MOUNT_PATHS.contains("/mcp"));
}

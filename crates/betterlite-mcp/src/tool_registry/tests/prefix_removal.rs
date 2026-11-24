use tracing::Level;

use super::support::{capture_logs, names, registry};

#[test]
fn removes_matching_tools_and_returns_the_count() {
    let loaded = registry(&["srv_a", "other", "srv_b"]);
    assert_eq!(loaded.unregister_tools_with_prefix("srv_"), 2);
    assert_eq!(names(&loaded), ["other"]);
}

#[test]
fn an_empty_prefix_removes_nothing() {
    let loaded = registry(&["srv_a", "other"]);
    assert_eq!(loaded.unregister_tools_with_prefix(""), 0);
    assert_eq!(names(&loaded), ["srv_a", "other"]);
}

#[test]
fn a_prefix_matching_nothing_removes_nothing() {
    let loaded = registry(&["srv_a", "other"]);
    assert_eq!(loaded.unregister_tools_with_prefix("none_"), 0);
    assert_eq!(names(&loaded), ["srv_a", "other"]);
}

#[test]
fn the_prefix_is_a_plain_starts_with() {
    let loaded = registry(&["srv", "srv_x_y", "other"]);
    assert_eq!(loaded.unregister_tools_with_prefix("srv_"), 1);
    assert_eq!(names(&loaded), ["srv", "other"]);
}

#[test]
fn logs_each_removal_at_debug() {
    let loaded = registry(&["srv_a", "srv_b"]);
    let (logs, _guard) = capture_logs();
    assert_eq!(loaded.unregister_tools_with_prefix("srv_"), 2);
    assert_eq!(
        logs.messages_at(Level::DEBUG),
        ["Unregistered MCP tool srv_a", "Unregistered MCP tool srv_b"]
    );
}

use super::support::{names, registry};

#[test]
fn lists_tools_in_insertion_order() {
    let loaded = registry(&["alpha", "beta", "gamma"]);
    assert_eq!(names(&loaded), ["alpha", "beta", "gamma"]);
}

#[test]
fn a_missing_prefix_lists_every_tool() {
    let loaded = registry(&["alpha", "beta"]);
    assert_eq!(names(&loaded), ["alpha", "beta"]);
    assert_eq!(loaded.list_tools(None).len(), 2);
}

#[test]
fn an_empty_prefix_lists_every_tool() {
    let loaded = registry(&["alpha", "beta"]);
    let listed = loaded.list_tools(Some(""));
    let found: Vec<&str> = listed.iter().map(|tool| tool.name.as_str()).collect();
    assert_eq!(found, ["alpha", "beta"]);
}

#[test]
fn a_prefix_filters_by_starts_with() {
    let loaded = registry(&["srv_a", "other", "srv_x_y"]);
    let listed = loaded.list_tools(Some("srv_"));
    let found: Vec<&str> = listed.iter().map(|tool| tool.name.as_str()).collect();
    assert_eq!(found, ["srv_a", "srv_x_y"]);
}

#[test]
fn a_prefix_is_case_sensitive() {
    let loaded = registry(&["srv_a"]);
    assert!(loaded.list_tools(Some("SRV_")).is_empty());
}

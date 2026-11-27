#![cfg(test)]

use std::collections::BTreeSet;

use betterlite_config::sections::mcp_env_values::StdioPolicy;

use crate::registry::tests::support::{record, registry};
use crate::server_utils::prefixes::PrefixPolicy;

use super::{ToolOwnerLookups as _, ToolOwners, server_exposes_tool};

const ALPHA: &str =
    r#"{"server_id":"test-a","name":"Alpha Server","transport":"http","alias":"alpha"}"#;
const BETA: &str =
    r#"{"server_id":"test-b","name":"Beta Server","transport":"http","server_name":"beta"}"#;
const SPACED: &str =
    r#"{"server_id":"test-c","name":"Spaced Name","transport":"http","alias":"my server"}"#;
const SHARED_FIRST: &str =
    r#"{"server_id":"test-a","name":"First","transport":"http","alias":"shared"}"#;
const SHARED_SECOND: &str =
    r#"{"server_id":"test-b","name":"Second","transport":"http","alias":"shared"}"#;
const BLANK_NAME: &str = r#"{"server_id":"test-blank","name":"","transport":"http"}"#;
const STDIO: &str =
    r#"{"server_id":"test-stdio","name":"Stdio Server","transport":"stdio","alias":"stdio"}"#;

fn policy() -> PrefixPolicy {
    PrefixPolicy::default()
}

fn owners(entries: &[(&str, &str)]) -> ToolOwners {
    let mapping = entries
        .iter()
        .map(|(tool, owner)| ((*tool).to_owned(), (*owner).to_owned()))
        .collect();
    ToolOwners { mapping }
}

const fn stdio(enabled: bool) -> StdioPolicy {
    StdioPolicy {
        enabled,
        allowed_commands: BTreeSet::new(),
    }
}

#[test]
fn prefix_map_normalizes_spaces() {
    let loaded = registry(&[SPACED], &[]);
    let map = loaded.known_prefix_to_server(&policy());
    assert!(map.contains_key("my_server"));
    assert!(!map.contains_key("my server"));
    assert_eq!(map["my_server"].fields.server_id, "test-c");
}

#[test]
fn prefix_map_keeps_a_shared_form_with_the_first_server() {
    let loaded = registry(&[SHARED_FIRST, SHARED_SECOND], &[]);
    let map = loaded.known_prefix_to_server(&policy());
    assert_eq!(map["shared"].fields.server_id, "test-a");
}

#[test]
fn prefix_map_has_the_alias_and_server_id_forms() {
    let loaded = registry(&[ALPHA], &[]);
    let map = loaded.known_prefix_to_server(&policy());
    assert_eq!(map["alpha"].fields.server_id, "test-a");
    assert_eq!(map["test-a"].fields.server_id, "test-a");
}

#[test]
fn exposes_tool_through_the_bare_spelling() {
    let known = owners(&[("echo", "Alpha Server")]);
    assert!(server_exposes_tool(
        &record(ALPHA),
        "echo",
        &known,
        &policy()
    ));
}

#[test]
fn exposes_tool_through_a_prefixed_spelling() {
    let known = owners(&[("alpha-echo", "Alpha Server")]);
    assert!(server_exposes_tool(
        &record(ALPHA),
        "echo",
        &known,
        &policy()
    ));
}

#[test]
fn exposes_tool_of_another_server_is_false() {
    let known = owners(&[("echo", "Beta Server")]);
    assert!(!server_exposes_tool(
        &record(ALPHA),
        "echo",
        &known,
        &policy()
    ));
}

#[test]
fn exposes_tool_normalizes_a_spaced_name() {
    let known = owners(&[("echo", "Spaced Name")]);
    assert!(server_exposes_tool(
        &record(SPACED),
        "echo",
        &known,
        &policy()
    ));
}

#[test]
fn exposes_tool_ignores_an_empty_owner() {
    let known = owners(&[("echo", "")]);
    assert!(!server_exposes_tool(
        &record(BLANK_NAME),
        "echo",
        &known,
        &policy()
    ));
}

#[test]
fn server_for_tool_name_finds_an_owner_prefix() {
    let loaded = registry(&[ALPHA], &[]);
    let known = owners(&[("echo", "alpha")]);
    let found = loaded
        .server_for_tool_name("echo", &known, &policy(), &stdio(true))
        .unwrap();
    assert_eq!(found.fields.server_id, "test-a");
}

#[test]
fn server_for_tool_name_finds_a_record_name() {
    let loaded = registry(&[ALPHA], &[]);
    let known = owners(&[("echo", "Alpha Server")]);
    let found = loaded
        .server_for_tool_name("echo", &known, &policy(), &stdio(true))
        .unwrap();
    assert_eq!(found.fields.server_id, "test-a");
}

#[test]
fn server_for_tool_name_uses_a_known_prefix() {
    let loaded = registry(&[ALPHA], &[]);
    let known = owners(&[("echo", "alpha")]);
    let found = loaded
        .server_for_tool_name("alpha-echo", &known, &policy(), &stdio(true))
        .unwrap();
    assert_eq!(found.fields.server_id, "test-a");
}

#[test]
fn server_for_tool_name_returns_a_blocked_stdio_server() {
    let loaded = registry(&[STDIO], &[]);
    let found = loaded
        .server_for_tool_name("stdio-echo", &owners(&[]), &policy(), &stdio(false))
        .unwrap();
    assert_eq!(found.fields.server_id, "test-stdio");
}

#[test]
fn server_for_tool_name_leaves_an_enabled_stdio_server() {
    let loaded = registry(&[STDIO], &[]);
    assert!(
        loaded
            .server_for_tool_name("stdio-echo", &owners(&[]), &policy(), &stdio(true))
            .is_none()
    );
}

#[test]
fn server_for_tool_name_unknown_is_none() {
    let loaded = registry(&[ALPHA], &[]);
    assert!(
        loaded
            .server_for_tool_name("unknown-tool", &owners(&[]), &policy(), &stdio(true))
            .is_none()
    );
}

#[test]
fn server_for_tool_name_ignores_an_owner_without_a_server() {
    let loaded = registry(&[ALPHA], &[]);
    let known = owners(&[("echo", "ghost")]);
    assert!(
        loaded
            .server_for_tool_name("echo", &known, &policy(), &stdio(true))
            .is_none()
    );
}

#[test]
fn server_owning_prefix_returns_the_matching_server() {
    let loaded = registry(&[ALPHA, BETA], &[]);
    let found = loaded.server_owning_prefix("beta-echo", &policy()).unwrap();
    assert_eq!(found.fields.server_id, "test-b");
    assert!(
        loaded
            .server_owning_prefix("gamma-echo", &policy())
            .is_none()
    );
}

#[test]
fn server_owning_prefix_has_no_owner_map() {
    let loaded = registry(&[ALPHA], &[]);
    let found = loaded
        .server_owning_prefix("alpha-echo", &policy())
        .unwrap();
    assert_eq!(found.fields.server_id, "test-a");
}

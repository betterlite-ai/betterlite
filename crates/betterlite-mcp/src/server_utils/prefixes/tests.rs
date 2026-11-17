#![cfg(test)]

use rstest::rstest;

use super::{
    PrefixError, PrefixPolicy, compute_short_server_prefix, get_server_prefix,
    iter_known_server_prefixes, match_known_server_prefix,
};
use crate::server_record::MCPServer;

fn server(json: &str) -> MCPServer {
    serde_json::from_str(json).unwrap()
}

fn base_server() -> MCPServer {
    server(r#"{"server_id": "srv", "name": "Server", "transport": "http"}"#)
}

fn short_enabled() -> PrefixPolicy {
    PrefixPolicy {
        separator: "-".to_owned(),
        short_prefix_enabled: true,
    }
}

#[rstest]
#[case::first_attempt("srv", 0, "tNu")]
#[case::second_attempt("srv", 1, "p13")]
#[case::third_attempt("srv", 2, "pKF")]
#[case::uuid("cd78779d-0596-4d16-9961-d2e340ef2827", 0, "t93")]
#[case::hyphenated("server-1", 0, "zin")]
#[case::single_character("x", 0, "wQu")]
fn compute_short_server_prefix_folds_the_sha256_of_the_seed(
    #[case] server_id: &str,
    #[case] attempt: u32,
    #[case] expected: &str,
) {
    assert_eq!(
        compute_short_server_prefix(server_id, attempt).unwrap(),
        expected
    );
}

#[test]
fn compute_short_server_prefix_requires_a_server_id() {
    assert_eq!(
        compute_short_server_prefix("", 0),
        Err(PrefixError::EmptyServerId)
    );
}

#[test]
fn get_server_prefix_takes_the_computed_hash_when_short_mode_is_on() {
    assert_eq!(get_server_prefix(&base_server(), &short_enabled()), "tNu");
}

#[test]
fn get_server_prefix_prefers_a_cached_short_prefix() {
    let mut with_cache = base_server();
    with_cache.fields.short_prefix = Some("cached".to_owned());
    assert_eq!(get_server_prefix(&with_cache, &short_enabled()), "cached");
}

#[test]
fn get_server_prefix_ignores_an_empty_cached_short_prefix() {
    let mut with_empty_cache = base_server();
    with_empty_cache.fields.short_prefix = Some(String::new());
    assert_eq!(
        get_server_prefix(&with_empty_cache, &short_enabled()),
        "tNu"
    );
}

#[test]
fn get_server_prefix_uses_the_alias_when_short_mode_is_off() {
    let mut aliased = base_server();
    aliased.fields.alias = Some("alias".to_owned());
    aliased.fields.server_name = Some("Server Name".to_owned());
    assert_eq!(
        get_server_prefix(&aliased, &PrefixPolicy::default()),
        "alias"
    );
}

#[test]
fn get_server_prefix_skips_an_empty_alias() {
    let mut empty_alias = base_server();
    empty_alias.fields.alias = Some(String::new());
    empty_alias.fields.server_name = Some("Server Name".to_owned());
    assert_eq!(
        get_server_prefix(&empty_alias, &PrefixPolicy::default()),
        "Server Name"
    );
}

#[test]
fn get_server_prefix_falls_back_to_the_server_name_then_the_id() {
    let mut named = base_server();
    named.fields.server_name = Some("Server Name".to_owned());
    assert_eq!(
        get_server_prefix(&named, &PrefixPolicy::default()),
        "Server Name"
    );
    assert_eq!(
        get_server_prefix(&base_server(), &PrefixPolicy::default()),
        "srv"
    );
}

#[test]
fn get_server_prefix_falls_back_when_short_mode_has_no_server_id() {
    let mut nameless = server(r#"{"server_id": "", "name": "Server", "transport": "http"}"#);
    nameless.fields.alias = Some("alias".to_owned());
    assert_eq!(get_server_prefix(&nameless, &short_enabled()), "alias");
}

#[test]
fn iter_known_server_prefixes_lists_every_spelling_once() {
    let mut aliased = base_server();
    aliased.fields.alias = Some("alias".to_owned());
    aliased.fields.server_name = Some("Server Name".to_owned());
    assert_eq!(
        iter_known_server_prefixes(&aliased, &PrefixPolicy::default()),
        vec![
            "alias".to_owned(),
            "tNu".to_owned(),
            "Server Name".to_owned(),
            "srv".to_owned(),
        ]
    );
}

#[test]
fn iter_known_server_prefixes_skips_empty_and_repeated_entries() {
    let repeated = server(
        r#"{"server_id": "srv", "name": "Server", "alias": "", "server_name": "srv", "short_prefix": "tNu", "transport": "http"}"#,
    );
    assert_eq!(
        iter_known_server_prefixes(&repeated, &short_enabled()),
        vec!["tNu".to_owned(), "srv".to_owned()]
    );
}

#[test]
fn iter_known_server_prefixes_keeps_the_short_prefix_even_with_short_mode_off() {
    let mut cached = base_server();
    cached.fields.short_prefix = Some("cached".to_owned());
    assert_eq!(
        iter_known_server_prefixes(&cached, &PrefixPolicy::default()),
        vec!["srv".to_owned(), "cached".to_owned(), "tNu".to_owned()]
    );
}

#[test]
fn match_known_server_prefix_returns_the_stripped_name() {
    let prefixes = vec!["srv".to_owned()];
    assert_eq!(
        match_known_server_prefix("srv-get_pets", &prefixes, &PrefixPolicy::default()),
        Some(("srv".to_owned(), "get_pets".to_owned()))
    );
}

#[test]
fn match_known_server_prefix_requires_the_separator() {
    let prefixes = vec!["srv".to_owned()];
    assert_eq!(
        match_known_server_prefix("srvget_pets", &prefixes, &PrefixPolicy::default()),
        None
    );
}

#[test]
fn match_known_server_prefix_honours_the_policy_separator() {
    let policy = PrefixPolicy {
        separator: ".".to_owned(),
        short_prefix_enabled: false,
    };
    let prefixes = vec!["srv".to_owned()];
    assert_eq!(
        match_known_server_prefix("srv.get_pets", &prefixes, &policy),
        Some(("srv".to_owned(), "get_pets".to_owned()))
    );
    assert_eq!(
        match_known_server_prefix("srv-get_pets", &prefixes, &policy),
        None
    );
}

#[test]
fn match_known_server_prefix_prefers_the_longest_prefix() {
    let prefixes = vec![
        "abcdef12".to_owned(),
        "abcdef12-3456-7890-abcd-ef1234567890".to_owned(),
    ];
    assert_eq!(
        match_known_server_prefix(
            "abcdef12-3456-7890-abcd-ef1234567890-get_pets",
            &prefixes,
            &PrefixPolicy::default()
        ),
        Some((
            "abcdef12-3456-7890-abcd-ef1234567890".to_owned(),
            "get_pets".to_owned()
        ))
    );
}

#[test]
fn match_known_server_prefix_normalizes_spaces_in_candidates() {
    let prefixes = vec!["Server Name".to_owned()];
    assert_eq!(
        match_known_server_prefix("Server_Name-get_pets", &prefixes, &PrefixPolicy::default()),
        Some(("Server_Name".to_owned(), "get_pets".to_owned()))
    );
    assert_eq!(
        match_known_server_prefix("Server Name-get_pets", &prefixes, &PrefixPolicy::default()),
        None
    );
}

#[test]
fn match_known_server_prefix_ignores_empty_and_duplicate_candidates() {
    let prefixes = vec![
        String::new(),
        "srv".to_owned(),
        "srv".to_owned(),
        String::new(),
    ];
    assert_eq!(
        match_known_server_prefix("srv-get_pets", &prefixes, &PrefixPolicy::default()),
        Some(("srv".to_owned(), "get_pets".to_owned()))
    );
}

#[test]
fn match_known_server_prefix_returns_none_for_an_unrelated_name() {
    let prefixes = vec!["srv".to_owned()];
    assert_eq!(
        match_known_server_prefix("other-get_pets", &prefixes, &PrefixPolicy::default()),
        None
    );
    assert_eq!(
        match_known_server_prefix("srv-get_pets", &[], &PrefixPolicy::default()),
        None
    );
}

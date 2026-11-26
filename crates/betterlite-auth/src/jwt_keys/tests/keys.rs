use base64::Engine as _;
use base64::engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD};
use betterlite_models::decisions::Json;

use crate::jwt_keys::agent::{AgentLookup as _, NoRegisteredAgents};
use crate::jwt_keys::resolver::keys::{is_allowed_domain, parse_keys, token_kid};

use super::support::{json, key, keys};

#[test]
fn key_dict_cardinality_and_last_matching_kid() {
    // a one-entry dict is a key candidate, and its kid is the only check
    let single = json(r#"{"kid":"a"}"#);
    assert_eq!(parse_keys(&single, Some("a")), Some(single.clone()));
    assert_eq!(parse_keys(&single, Some("b")), None);
    assert_eq!(parse_keys(&single, None), Some(single.clone()));

    // a dict with more than one field yields nothing, even when its kid matches
    assert_eq!(parse_keys(&key("a"), Some("a")), None);
    assert_eq!(parse_keys(&key("a"), None), None);

    // a one-entry list is checked on its only entry, and that entry must be an object
    assert_eq!(parse_keys(&keys(&[key("a")]), Some("a")), Some(key("a")));
    assert_eq!(parse_keys(&keys(&[key("a")]), Some("b")), None);
    assert_eq!(parse_keys(&keys(&[key("a")]), None), Some(key("a")));
    assert_eq!(parse_keys(&json(r#"["a"]"#), None), None);
    assert_eq!(parse_keys(&json("[7]"), None), None);

    // a longer list takes the last matching entry and needs a kid to match at all
    let listed = json(r#"[{"kid":"a","n":"1"},{"kid":"b","n":"2"},{"kid":"a","n":"3"}]"#);
    assert_eq!(
        parse_keys(&listed, Some("a")),
        Some(json(r#"{"kid":"a","n":"3"}"#))
    );
    assert_eq!(
        parse_keys(&listed, Some("b")),
        Some(json(r#"{"kid":"b","n":"2"}"#))
    );
    assert_eq!(parse_keys(&listed, Some("c")), None);
    assert_eq!(parse_keys(&listed, None), None);
    let mixed = json(r#"[{"kid":"a","n":"1"},7,"x",{"kid":"a","n":"2"}]"#);
    assert_eq!(
        parse_keys(&mixed, Some("a")),
        Some(json(r#"{"kid":"a","n":"2"}"#))
    );

    // nothing at all is nothing
    assert_eq!(parse_keys(&json("[]"), Some("a")), None);
    assert_eq!(parse_keys(&json("{}"), Some("a")), None);
    assert_eq!(parse_keys(&Json::Null, None), None);
    assert_eq!(parse_keys(&Json::Text("a".to_owned()), None), None);
}

#[test]
fn email_last_segment_case_sensitive() {
    assert!(is_allowed_domain(None, "anyone@example.org"));
    assert!(is_allowed_domain(Some("example.com"), "one@example.com"));
    assert!(!is_allowed_domain(
        Some("example.com"),
        "one@sub.example.com"
    ));
    assert!(!is_allowed_domain(Some("example.com"), "one@Example.com"));
    assert!(!is_allowed_domain(Some("Example.com"), "one@example.com"));
    assert!(is_allowed_domain(
        Some("example.com"),
        "weird@name@example.com"
    ));
    assert!(!is_allowed_domain(
        Some("example.com"),
        "weird@name@other.com"
    ));
    assert!(is_allowed_domain(Some("example.com"), "example.com"));
    assert!(!is_allowed_domain(Some("example.com"), "example.com.evil"));
    assert!(!is_allowed_domain(Some("example.com"), "one@example.com "));
    assert!(is_allowed_domain(Some(""), "one@"));
}

#[test]
fn no_registered_agents_defaults() {
    let lookup = NoRegisteredAgents;
    assert!(lookup.get_agent_list().is_empty());
    assert!(lookup.get_agent_by_id("agent-1").is_none());
    assert!(lookup.get_agent_by_name("agent-one").is_none());
}

fn compact(header: &str) -> String {
    format!("{header}.e30.c2ln")
}

#[test]
fn token_kid_reads_only_a_compact_jwt_object_header() {
    let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","kid":"a"}"#);
    assert_eq!(token_kid(&compact(&header)).as_deref(), Some("a"));

    // padding is what pyjwt strips, up to two characters
    let padded = URL_SAFE.encode(br#"{"kid":"a","n":1}"#);
    assert!(padded.ends_with("=="));
    assert_eq!(token_kid(&compact(&padded)).as_deref(), Some("a"));
    assert_eq!(token_kid(&compact(&format!("{padded}="))), None);

    // a header that is not a JSON object, has no kid, or has a non-string kid names no key
    for raw in [
        URL_SAFE_NO_PAD.encode(b"[1]"),
        URL_SAFE_NO_PAD.encode(b"\"kid\""),
        URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256"}"#),
        URL_SAFE_NO_PAD.encode(br#"{"kid":7}"#),
        URL_SAFE_NO_PAD.encode(br#"{"kid":null}"#),
        "!!!".to_owned(),
    ] {
        assert_eq!(token_kid(&compact(&raw)), None, "header {raw}");
    }

    // only a compact JWT has a header to read
    assert_eq!(token_kid("only.two"), None);
    assert_eq!(token_kid("one.two.three.four"), None);
    assert_eq!(token_kid(""), None);
    assert_eq!(token_kid(".."), None);
}

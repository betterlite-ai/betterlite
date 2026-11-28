use base64::Engine as _;
use base64::engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD};
use rstest::rstest;

use crate::jwt_keys::agent::{AgentLookup as _, NoRegisteredAgents};
use crate::jwt_keys::resolver::keys::{is_allowed_domain, parse_keys, token_kid};

use super::support::json;

#[rstest]
#[case::dict_match(r#"{"kid":"a"}"#, Some("a"), Some(r#"{"kid":"a"}"#))]
#[case::dict_miss(r#"{"kid":"a"}"#, Some("b"), None)]
#[case::dict_no_kid(r#"{"kid":"a"}"#, None, Some(r#"{"kid":"a"}"#))]
#[case::long_dict(r#"{"kid":"a","n":"1"}"#, Some("a"), None)]
#[case::long_dict_no_kid(r#"{"kid":"a","n":"1"}"#, None, None)]
#[case::list_match(r#"[{"kid":"a","n":"1"}]"#, Some("a"), Some(r#"{"kid":"a","n":"1"}"#))]
#[case::list_miss(r#"[{"kid":"a"}]"#, Some("b"), None)]
#[case::list_no_kid(r#"[{"kid":"a"}]"#, None, Some(r#"{"kid":"a"}"#))]
#[case::list_string(r#"["a"]"#, None, None)]
#[case::list_number("[7]", None, None)]
#[case::last_match(
    r#"[{"kid":"a","n":"1"},{"kid":"b","n":"2"},{"kid":"a","n":"3"}]"#,
    Some("a"),
    Some(r#"{"kid":"a","n":"3"}"#)
)]
#[case::middle_match(
    r#"[{"kid":"a"},{"kid":"b"},{"kid":"a"}]"#,
    Some("b"),
    Some(r#"{"kid":"b"}"#)
)]
#[case::long_list_miss(r#"[{"kid":"a"},{"kid":"b"}]"#, Some("c"), None)]
#[case::long_list_no_kid(r#"[{"kid":"a"},{"kid":"b"}]"#, None, None)]
#[case::mixed_list(
    r#"[{"kid":"a"},7,"x",{"kid":"a","n":"2"}]"#,
    Some("a"),
    Some(r#"{"kid":"a","n":"2"}"#)
)]
#[case::empty_list("[]", Some("a"), None)]
#[case::empty_object("{}", Some("a"), None)]
#[case::null("null", None, None)]
#[case::text(r#""a""#, None, None)]
fn key_dict_cardinality_and_last_matching_kid(
    #[case] body: &str,
    #[case] kid: Option<&str>,
    #[case] expected: Option<&str>,
) {
    assert_eq!(parse_keys(&json(body), kid), expected.map(json));
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
    let padded = URL_SAFE.encode(br#"{"kid":"a","n":100}"#);
    assert!(padded.ends_with("=="));
    assert_eq!(token_kid(&compact(&padded)).as_deref(), Some("a"));
    assert_eq!(token_kid(&compact(&format!("{padded}="))), None);

    // a header segment that is not base64 names no key
    assert_eq!(token_kid(&compact("!!!")), None);

    // only a compact JWT has a header to read
    assert_eq!(token_kid("only.two"), None);
    assert_eq!(token_kid("one.two.three.four"), None);
    assert_eq!(token_kid(""), None);
    assert_eq!(token_kid(".."), None);
}

#[rstest]
#[case::unpadded_short(r#"{"kid":"a"}"#, "", Some("a"))]
#[case::unpadded_quad(r#"{"kid":"aa"}"#, "", Some("aa"))]
#[case::unpadded_long(r#"{"kid":"aaa"}"#, "", Some("aaa"))]
#[case::single_padding(r#"{"kid":"a"}"#, "=", Some("a"))]
#[case::double_padding(r#"{"kid":"aaa"}"#, "==", Some("aaa"))]
#[case::incomplete_padding(r#"{"kid":"aaa"}"#, "=", None)]
#[case::extra_padding(r#"{"kid":"a"}"#, "==", None)]
#[case::unneeded_single_padding(r#"{"kid":"aa"}"#, "=", None)]
#[case::unneeded_double_padding(r#"{"kid":"aa"}"#, "==", None)]
#[case::excessive_padding(r#"{"kid":"aaa"}"#, "===", None)]
fn token_kid_requires_whole_quad_padding(
    #[case] header: &str,
    #[case] padding: &str,
    #[case] expected: Option<&str>,
) {
    let encoded = URL_SAFE_NO_PAD.encode(header.as_bytes());
    let token = format!("{encoded}{padding}.e30.c2ln");
    assert_eq!(token_kid(&token).as_deref(), expected);
}

#[rstest]
#[case::array("[1]")]
#[case::string(r#""kid""#)]
#[case::no_kid(r#"{"alg":"RS256"}"#)]
#[case::numeric_kid(r#"{"kid":7}"#)]
#[case::null_kid(r#"{"kid":null}"#)]
fn token_kid_rejects_unusable_headers(#[case] header: &str) {
    // a header that is not a JSON object, has no kid, or has a non-string kid names no key
    let raw = URL_SAFE_NO_PAD.encode(header.as_bytes());
    assert_eq!(token_kid(&compact(&raw)), None, "header {raw}");
}

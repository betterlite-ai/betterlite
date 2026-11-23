#![cfg(test)]

use betterlite_core::agents::json::AgentJson;
use rstest::rstest;

use super::{
    fix_agent_card_url, get_agent_card_url, is_localhost_or_internal_url, normalized_card,
    set_agent_card_url,
};

fn json(text: &str) -> AgentJson {
    match serde_json::from_str(text) {
        Ok(value) => value,
        Err(error) => panic!("the fixture is not JSON: {error}"),
    }
}

fn url_of(card: &AgentJson) -> AgentJson {
    match card.get("url") {
        Some(url) => url.clone(),
        None => AgentJson::Null,
    }
}

fn interface_url_of(card: &AgentJson) -> AgentJson {
    let Some(AgentJson::Array(mut entries)) = card.get("supportedInterfaces").cloned() else {
        return AgentJson::Null;
    };
    let Some(AgentJson::Object(mut entry)) = entries.pop() else {
        return AgentJson::Null;
    };
    entry.remove("url").unwrap_or(AgentJson::Null)
}

#[test]
fn card_url_reads_the_top_level_url_first() {
    let text = r#"{"name":"a","url":"https://top.example/","supportedInterfaces":[{"url":"https://interface.example/","protocolBinding":"JSONRPC"}]}"#;
    assert_eq!(
        get_agent_card_url(text).unwrap().as_deref(),
        Some("https://top.example/")
    );
}

#[test]
fn card_url_falls_back_to_the_first_interface() {
    let text = r#"{"name":"a","supportedInterfaces":[{"url":"https://first.example/","protocolBinding":"JSONRPC"},{"url":"https://second.example/","protocolBinding":"JSONRPC"}]}"#;
    assert_eq!(
        get_agent_card_url(text).unwrap().as_deref(),
        Some("https://first.example/")
    );
}

#[test]
fn card_url_ignores_an_empty_top_level_url() {
    let text =
        r#"{"name":"a","url":"","supportedInterfaces":[{"url":"https://interface.example/"}]}"#;
    assert_eq!(
        get_agent_card_url(text).unwrap().as_deref(),
        Some("https://interface.example/")
    );
}

#[test]
fn card_url_is_none_without_a_readable_url() {
    assert!(get_agent_card_url(r#"{"name":"a"}"#).unwrap().is_none());
    assert!(
        get_agent_card_url(r#"{"name":"a","url":"","supportedInterfaces":[]}"#)
            .unwrap()
            .is_none()
    );
}

#[test]
fn card_url_writes_a_trailing_slash_to_both_present_fields() {
    let text = r#"{"name":"a","url":"https://old.example/","supportedInterfaces":[{"url":"https://old.example/","protocolBinding":"JSONRPC"}]}"#;
    let written = json(&set_agent_card_url(text, "https://new.example").unwrap());
    assert_eq!(url_of(&written), json(r#""https://new.example/""#));
    assert_eq!(
        interface_url_of(&written),
        json(r#""https://new.example/""#)
    );
}

#[test]
fn card_url_leaves_absent_fields_absent() {
    let written = json(&set_agent_card_url(r#"{"name":"a"}"#, "https://new.example///").unwrap());
    assert_eq!(url_of(&written), AgentJson::Null);
    assert!(written.get("supportedInterfaces").is_none());
}

#[test]
fn card_url_reports_a_passthrough_interface_list_as_absent() {
    let text = r#"{"name":"a","supportedInterfaces":"none"}"#;
    assert_eq!(get_agent_card_url(text).unwrap(), None);
    let written = json(&set_agent_card_url(text, "https://new.example").unwrap());
    assert_eq!(written.get("supportedInterfaces"), Some(&json(r#""none""#)));
}

#[test]
fn card_url_rejects_text_that_is_not_a_card() {
    get_agent_card_url("{").unwrap_err();
    set_agent_card_url("{", "https://a/").unwrap_err();
    fix_agent_card_url("[]", "https://a/").unwrap_err();
}

#[rstest]
#[case::absent(None, false)]
#[case::empty(Some(""), false)]
#[case::localhost(Some("http://LOCALHOST:9/"), true)]
#[case::loopback(Some("http://127.0.0.1:8080/"), true)]
#[case::any(Some("http://0.0.0.0:80/"), true)]
#[case::ipv6(Some("http://[::1]:80/"), true)]
#[case::public(Some("https://agent.example"), false)]
fn url_is_localhost_or_internal_only_for_internal_hosts(
    #[case] url: Option<&str>,
    #[case] expected: bool,
) {
    assert_eq!(is_localhost_or_internal_url(url), expected);
}

#[test]
fn card_normalization_canonicalizes_the_protocol_binding() {
    let text = r#"{"name":"a","supportedInterfaces":[{"url":"https://a.example/","protocolBinding":"jsonrpc","protocolVersion":"1.0"}]}"#;
    let read = json(&normalized_card(text).unwrap());
    let interface = read
        .get("supportedInterfaces")
        .and_then(AgentJson::as_array)
        .and_then(|entries| entries.first())
        .unwrap();
    assert_eq!(
        interface.get("protocolBinding").and_then(AgentJson::as_str),
        Some("JSONRPC")
    );
    assert_eq!(
        interface.get("protocolVersion").and_then(AgentJson::as_str),
        Some("0.3")
    );
}

#[test]
fn card_normalization_leaves_a_card_without_interfaces_alone() {
    let text = r#"{"name":"a","url":"https://a.example/"}"#;
    assert_eq!(json(&normalized_card(text).unwrap()), json(text));
}

#[test]
fn card_url_fixes_a_localhost_card_through_the_function() {
    let text = r#"{"name":"a","url":"http://localhost:9/"}"#;
    let read = json(&fix_agent_card_url(text, "https://agent.example").unwrap());
    assert_eq!(
        read.get("url").and_then(AgentJson::as_str),
        Some("https://agent.example/")
    );
}

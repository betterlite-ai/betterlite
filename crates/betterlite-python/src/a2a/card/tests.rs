#![cfg(test)]

use betterlite_a2a::agent_card::AgentCard;
use betterlite_core::agents::json::AgentJson;
use rstest::rstest;

use super::{fix, parse, text};

const BASE: &str = "https://agent.example";
const FIXED: &str = r#""https://agent.example/""#;

fn json(text: &str) -> AgentJson {
    match serde_json::from_str(text) {
        Ok(value) => value,
        Err(error) => panic!("the fixture is not JSON: {error}"),
    }
}

fn fixture_card(url: Option<&str>, interface: Option<&str>) -> AgentCard {
    let mut fields = vec![String::from(r#""name":"a""#)];
    if let Some(url) = url {
        fields.push(format!(r#""url":{url}"#));
    }
    if let Some(interface) = interface {
        fields.push(format!(
            r#""supportedInterfaces":[{{"url":{interface},"protocolBinding":"JSONRPC"}}]"#
        ));
    }
    let body = fields.join(",");
    match parse(&format!("{{{body}}}")) {
        Ok(card) => card,
        Err(error) => panic!("the fixture card is not valid: {error}"),
    }
}

fn written(card: &AgentCard) -> AgentJson {
    match text(card) {
        Ok(json) => json(&json),
        Err(error) => panic!("the card does not serialize: {error}"),
    }
}

fn field(card: &AgentCard, key: &str) -> AgentJson {
    match written(card).get(key) {
        Some(value) => value.clone(),
        None => AgentJson::Null,
    }
}

fn interface_url(card: &AgentCard) -> AgentJson {
    let AgentJson::Array(mut entries) = field(card, "supportedInterfaces") else {
        return AgentJson::Null;
    };
    let Some(AgentJson::Object(mut entry)) = entries.pop() else {
        return AgentJson::Null;
    };
    entry.remove("url").unwrap_or(AgentJson::Null)
}

#[test]
fn card_parse_round_trips_every_wire_field() {
    let source = r#"{"protocolVersion":"1.0","name":"agent","description":null,"url":"https://a.example/","capabilities":{"streaming":true},"defaultInputModes":["text"],"defaultOutputModes":["text"],"skills":[],"supportedInterfaces":[{"url":"https://a.example/","protocolBinding":"jsonrpc","protocolVersion":"1.0"}],"x-unknown":{"kept":true}}"#;
    let card = match parse(source) {
        Ok(card) => card,
        Err(error) => panic!("the fixture card is not valid: {error}"),
    };
    assert_eq!(written(&card), json(source));
}

#[test]
fn card_parse_rejects_text_that_is_not_a_card() {
    parse("{").unwrap_err();
    parse("[]").unwrap_err();
}

#[rstest]
#[case::both_localhost(
    Some(r#""http://localhost:9/""#),
    Some(r#""http://localhost:9/""#),
    true,
    true
)]
#[case::public_card_localhost_interface(
    Some(r#""https://public.example/""#),
    Some(r#""http://127.0.0.1:9/""#),
    false,
    true
)]
#[case::localhost_card_public_interface(
    Some(r#""http://0.0.0.0:9/""#),
    Some(r#""https://public.example/""#),
    true,
    false
)]
#[case::both_public(
    Some(r#""https://public.example/""#),
    Some(r#""https://interface.example/""#),
    false,
    false
)]
#[case::empty_interface_url(Some(r#""http://localhost:9/""#), Some(r#""""#), true, false)]
fn card_fix_rewrites_only_the_localhost_fields(
    #[case] url: Option<&str>,
    #[case] interface: Option<&str>,
    #[case] fix_card: bool,
    #[case] fix_interface: bool,
) {
    let mut found = fixture_card(url, interface);
    fix(&mut found, BASE);
    let expected_card = if fix_card {
        json(FIXED)
    } else {
        json(url.unwrap_or("null"))
    };
    assert_eq!(field(&found, "url"), expected_card);
    let expected_interface = if fix_interface {
        json(FIXED)
    } else {
        json(interface.unwrap_or("null"))
    };
    assert_eq!(interface_url(&found), expected_interface);
}

#[test]
fn card_fix_leaves_an_absent_url_absent() {
    let mut found = fixture_card(None, Some(r#""http://localhost:9/""#));
    fix(&mut found, BASE);
    assert_eq!(field(&found, "url"), AgentJson::Null);
    assert_eq!(interface_url(&found), json(FIXED));
}

#[test]
fn card_fix_normalizes_the_base_url_to_one_trailing_slash() {
    let mut found = fixture_card(Some(r#""http://localhost:9/""#), None);
    fix(&mut found, "https://agent.example///");
    assert_eq!(field(&found, "url"), json(FIXED));
}

#[test]
fn card_fix_ignores_a_passthrough_interface_list() {
    let mut found = match parse(r#"{"supportedInterfaces":"none"}"#) {
        Ok(card) => card,
        Err(error) => panic!("the fixture card is not valid: {error}"),
    };
    fix(&mut found, BASE);
    assert_eq!(field(&found, "supportedInterfaces"), json(r#""none""#));
}

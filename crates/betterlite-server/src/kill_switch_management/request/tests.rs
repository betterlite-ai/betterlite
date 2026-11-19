#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_core::agents::json::AgentJson;
use betterlite_core::agents::{AgentKillSwitchAuth, AgentKillSwitchConfig, AgentKillSwitchMethod};
use rstest::rstest;
use secrecy::ExposeSecret as _;
use url::ParseError;

use super::{KillSwitchRequest, KillSwitchRequestError, build_kill_switch_request, method_text};
use crate::kill_switch_management::support::{
    api_key_auth, basic_auth, bearer_auth, config_with_auth, default_api_key_auth,
};

fn config_at(url: &str) -> AgentKillSwitchConfig {
    let mut config = config_with_auth(None);
    config.url = String::from(url);
    config
}

fn request_for(config: &AgentKillSwitchConfig) -> KillSwitchRequest {
    build_kill_switch_request(config).expect("the request is built")
}

fn header_values(request: &KillSwitchRequest) -> BTreeMap<String, String> {
    request
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), value.expose_secret().clone()))
        .collect()
}

#[test]
fn a_request_keeps_the_configured_method_body_and_url() {
    let mut config = config_with_auth(None);
    config.method = AgentKillSwitchMethod::Patch;
    config.body = Some(BTreeMap::from([(
        String::from("reason"),
        AgentJson::String(String::from("kill")),
    )]));
    let request = request_for(&config);
    assert_eq!(request.method, AgentKillSwitchMethod::Patch);
    assert_eq!(request.url, "https://agent.example/stop");
    assert_eq!(request.reported_url, "https://agent.example/stop");
    assert_eq!(request.body, config.body);
    assert!(request.headers.is_empty());
}

#[test]
fn an_empty_query_section_is_dropped() {
    let config = config_at("https://agent.example/stop?");
    assert_eq!(request_for(&config).url, "https://agent.example/stop");
}

#[test]
fn a_configured_param_replaces_the_url_value_in_place() {
    let mut config = config_at("https://agent.example/stop?region=us&token=old");
    config
        .query_params
        .insert(String::from("region"), String::from("eu"));
    assert_eq!(
        request_for(&config).url,
        "https://agent.example/stop?region=eu&token=old"
    );
}

#[test]
fn a_new_param_is_appended_after_the_url_params() {
    let mut config = config_at("https://agent.example/stop?a=1");
    config
        .query_params
        .insert(String::from("b"), String::from("2"));
    assert_eq!(
        request_for(&config).url,
        "https://agent.example/stop?a=1&b=2"
    );
}

#[test]
fn repeated_query_keys_are_grouped_at_their_first_position() {
    let config = config_at("https://agent.example/stop?a=1&b=2&a=3");
    assert_eq!(
        request_for(&config).url,
        "https://agent.example/stop?a=1&a=3&b=2"
    );
}

#[test]
fn an_existing_query_value_is_decoded_and_encoded_again() {
    let config = config_at("https://agent.example/stop?q=%2A&r=a+b");
    assert_eq!(
        request_for(&config).url,
        "https://agent.example/stop?q=%2A&r=a+b"
    );
}

#[rstest]
#[case::asterisk("*", "%2A")]
#[case::tilde("~", "~")]
#[case::space("a b", "a+b")]
#[case::plus("a+b", "a%2Bb")]
#[case::slash("/", "%2F")]
#[case::non_ascii("é", "%C3%A9")]
fn a_query_value_is_encoded_like_python_quote_plus(#[case] value: &str, #[case] encoded: &str) {
    let mut config = config_at("https://agent.example/stop");
    config
        .query_params
        .insert(String::from("q"), String::from(value));
    assert_eq!(
        request_for(&config).url,
        format!("https://agent.example/stop?q={encoded}")
    );
}

#[test]
fn the_fragment_survives_the_merge_and_is_left_out_of_the_reported_url() {
    let mut config = config_at("https://agent.example/stop?a=1#done");
    config
        .query_params
        .insert(String::from("b"), String::from("2"));
    let request = request_for(&config);
    assert_eq!(request.url, "https://agent.example/stop?a=1&b=2#done");
    assert_eq!(request.reported_url, "https://agent.example/stop#done");
}

#[test]
fn the_url_is_normalized_by_the_standard_parser() {
    let config = config_at("HTTPS://Agent.Example:443/stop");
    assert_eq!(request_for(&config).url, "https://agent.example/stop");
}

#[test]
fn a_bearer_auth_sends_the_token_in_the_authorization_header() {
    let config = config_with_auth(Some(bearer_auth("sk-live")));
    assert_eq!(
        header_values(&request_for(&config)),
        BTreeMap::from([(
            String::from("Authorization"),
            String::from("Bearer sk-live")
        )])
    );
}

#[test]
fn an_api_key_auth_sends_the_key_under_the_default_header_name() {
    let config = config_with_auth(Some(default_api_key_auth("key-live")));
    assert_eq!(
        header_values(&request_for(&config)),
        BTreeMap::from([(String::from("x-api-key"), String::from("key-live"))])
    );
}

#[test]
fn an_api_key_auth_sends_the_key_under_the_configured_header_name() {
    let config = config_with_auth(Some(api_key_auth("x-custom-key", "key-live")));
    assert_eq!(
        header_values(&request_for(&config)),
        BTreeMap::from([(String::from("x-custom-key"), String::from("key-live"))])
    );
}

#[test]
fn a_basic_auth_sends_the_base64_encoded_credentials() {
    let config = config_with_auth(Some(basic_auth("user", "pass")));
    assert_eq!(
        header_values(&request_for(&config)),
        BTreeMap::from([(
            String::from("Authorization"),
            String::from("Basic dXNlcjpwYXNz")
        )])
    );
}

#[test]
fn configured_headers_are_sent_alongside_the_auth_header() {
    let mut config = config_with_auth(Some(bearer_auth("sk-live")));
    config
        .headers
        .insert(String::from("x-trace"), String::from("on"));
    assert_eq!(
        header_values(&request_for(&config)),
        BTreeMap::from([
            (
                String::from("Authorization"),
                String::from("Bearer sk-live")
            ),
            (String::from("x-trace"), String::from("on")),
        ])
    );
}

#[rstest]
#[case::bearer(bearer_auth("tok"), "Authorization", "Bearer tok")]
#[case::basic(basic_auth("user", "pass"), "Authorization", "Basic dXNlcjpwYXNz")]
#[case::api_key(api_key_auth("x-api-key", "key"), "x-api-key", "key")]
fn the_auth_header_replaces_a_configured_header_of_the_same_name(
    #[case] auth: AgentKillSwitchAuth,
    #[case] name: &str,
    #[case] value: &str,
) {
    let mut config = config_with_auth(Some(auth));
    config
        .headers
        .insert(String::from(name), String::from("stale"));
    let headers = header_values(&request_for(&config));
    assert_eq!(headers.get(name).map(String::as_str), Some(value));
    assert_eq!(headers.len(), 1);
}

#[rstest]
#[case::lower("authorization")]
#[case::upper("AUTHORIZATION")]
#[case::mixed("AuThOrIzAtIoN")]
fn the_auth_header_replaces_a_configured_header_whatever_its_case(#[case] name: &str) {
    let mut config = config_with_auth(Some(bearer_auth("tok")));
    config
        .headers
        .insert(String::from(name), String::from("stale"));
    let headers = header_values(&request_for(&config));
    assert_eq!(headers.len(), 1);
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer tok")
    );
}

#[test]
fn no_auth_sends_no_authorization_header() {
    let mut config = config_with_auth(None);
    config
        .headers
        .insert(String::from("x-trace"), String::from("on"));
    assert_eq!(
        header_values(&request_for(&config)),
        BTreeMap::from([(String::from("x-trace"), String::from("on"))])
    );
}

#[rstest]
#[case::post(AgentKillSwitchMethod::Post, "POST")]
#[case::put(AgentKillSwitchMethod::Put, "PUT")]
#[case::patch(AgentKillSwitchMethod::Patch, "PATCH")]
#[case::delete(AgentKillSwitchMethod::Delete, "DELETE")]
#[case::get(AgentKillSwitchMethod::Get, "GET")]
fn method_text_maps_every_method(#[case] method: AgentKillSwitchMethod, #[case] text: &str) {
    assert_eq!(method_text(&method), text);
}

#[test]
fn an_invalid_url_is_a_request_error() {
    let config = config_at("not a url");
    let error = build_kill_switch_request(&config).expect_err("the URL is refused");
    let KillSwitchRequestError::Url(source) = error else {
        panic!("the error names the URL");
    };
    assert_eq!(source, ParseError::RelativeUrlWithoutBase);
}

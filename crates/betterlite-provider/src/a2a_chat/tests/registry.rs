use std::collections::BTreeMap;
use std::sync::Mutex;

use betterlite_prompts::arguments::json::Object;
use rstest::rstest;
use secrecy::ExposeSecret;

use crate::a2a_chat::error::A2aFailure;
use crate::a2a_chat::registry::{
    A2aAgentLookup, RegisteredA2aAgent, RegistryInput, RegistryResolved, resolve_agent_config,
};

use super::support::{FakeEntra, RefusingEntra, header_map, object, secret};

fn agent(card: &str, params: &str) -> RegisteredA2aAgent {
    RegisteredA2aAgent {
        agent_card_params: object(card),
        litellm_params: object(params),
    }
}

struct FakeLookup {
    agent: Option<RegisteredA2aAgent>,
    names: Mutex<Vec<String>>,
}

impl FakeLookup {
    fn new(agent: Option<RegisteredA2aAgent>) -> Self {
        Self {
            agent,
            names: Mutex::new(Vec::new()),
        }
    }

    fn names(&self) -> Vec<String> {
        self.names.lock().unwrap().clone()
    }
}

impl A2aAgentLookup for FakeLookup {
    fn agent_by_name(&self, name: &str) -> Option<RegisteredA2aAgent> {
        self.names.lock().unwrap().push(name.to_owned());
        self.agent.clone()
    }
}

fn resolve(
    input: &RegistryInput<'_>,
    optional_params: &mut Object,
    lookup: &dyn A2aAgentLookup,
) -> Result<RegistryResolved, A2aFailure> {
    resolve_agent_config(input, optional_params, Some(lookup), &FakeEntra)
}

fn api_key_of(resolved: &RegistryResolved) -> Option<&str> {
    resolved.api_key.as_ref().map(ExposeSecret::expose_secret)
}

#[test]
fn an_empty_agent_name_returns_the_inputs_untouched() {
    let lookup = FakeLookup::new(Some(agent(
        r#"{"url":"http://card"}"#,
        r#"{"api_key":"stored"}"#,
    )));
    let mut optional_params = Object::new();
    let input = RegistryInput {
        agent_name: "",
        api_base: None,
        api_key: None,
        headers: None,
    };
    let resolved = resolve(&input, &mut optional_params, &lookup).unwrap();
    assert_eq!(resolved.api_base, None);
    assert_eq!(api_key_of(&resolved), None);
    assert_eq!(resolved.headers, None);
    assert!(optional_params.is_empty());
    assert!(lookup.names().is_empty());
}

#[test]
fn an_explicit_base_key_and_headers_skip_the_lookup() {
    let lookup = FakeLookup::new(Some(agent(r#"{"url":"http://card"}"#, r#"{"timeout":7}"#)));
    let key = secret("explicit-key");
    let headers = header_map(&[("h", "v")]);
    let mut optional_params = Object::new();
    let input = RegistryInput {
        agent_name: "agent",
        api_base: Some("http://explicit"),
        api_key: Some(&key),
        headers: Some(&headers),
    };
    let resolved = resolve(&input, &mut optional_params, &lookup).unwrap();
    assert_eq!(resolved.api_base, Some(String::from("http://explicit")));
    assert_eq!(api_key_of(&resolved), Some("explicit-key"));
    assert_eq!(resolved.headers, Some(headers));
    assert!(optional_params.is_empty());
    assert!(lookup.names().is_empty());
}

#[test]
fn empty_headers_still_ask_the_lookup() {
    let lookup = FakeLookup::new(Some(agent("{}", r#"{"headers":{"X":"1"},"timeout":7}"#)));
    let key = secret("caller-key");
    let empty = BTreeMap::new();
    let mut optional_params = Object::new();
    let input = RegistryInput {
        agent_name: "agent",
        api_base: None,
        api_key: Some(&key),
        headers: Some(&empty),
    };
    let resolved = resolve(&input, &mut optional_params, &lookup).unwrap();
    assert_eq!(lookup.names(), vec![String::from("agent")]);
    assert_eq!(resolved.headers, Some(header_map(&[("X", "1")])));
    assert_eq!(api_key_of(&resolved), Some("caller-key"));
    assert_eq!(optional_params, object(r#"{"timeout":7}"#));
}

#[test]
fn no_lookup_returns_the_inputs_untouched() {
    let mut optional_params = Object::new();
    let input = RegistryInput {
        agent_name: "agent",
        api_base: Some("http://caller"),
        api_key: None,
        headers: None,
    };
    let resolved = resolve_agent_config(&input, &mut optional_params, None, &FakeEntra).unwrap();
    assert_eq!(resolved.api_base, Some(String::from("http://caller")));
    assert_eq!(api_key_of(&resolved), None);
    assert!(optional_params.is_empty());
}

#[test]
fn an_unknown_agent_returns_the_inputs_untouched() {
    let lookup = FakeLookup::new(None);
    let mut optional_params = Object::new();
    let input = RegistryInput {
        agent_name: "agent",
        api_base: Some("http://caller"),
        api_key: None,
        headers: None,
    };
    let resolved = resolve(&input, &mut optional_params, &lookup).unwrap();
    assert_eq!(lookup.names(), vec![String::from("agent")]);
    assert_eq!(resolved.api_base, Some(String::from("http://caller")));
    assert!(optional_params.is_empty());
}

#[rstest]
#[case::from_card(None, Some("http://card"), r#"{"url":"http://card"}"#)]
#[case::explicit_base_kept(
    Some("http://caller"),
    Some("http://caller"),
    r#"{"url":"http://card"}"#
)]
#[case::empty_card(None, None, "{}")]
#[case::card_without_url(None, None, r#"{"name":"x"}"#)]
fn the_card_url_fills_only_a_missing_api_base(
    #[case] given: Option<&str>,
    #[case] expected: Option<&str>,
    #[case] card: &str,
) {
    let lookup = FakeLookup::new(Some(agent(card, "{}")));
    let mut optional_params = Object::new();
    let input = RegistryInput {
        agent_name: "agent",
        api_base: given,
        api_key: None,
        headers: None,
    };
    let resolved = resolve(&input, &mut optional_params, &lookup).unwrap();
    assert_eq!(resolved.api_base.as_deref(), expected);
}

#[rstest]
#[case::string_key(r#"{"api_key":"stored"}"#, Some("stored"))]
#[case::empty_string_key(r#"{"api_key":""}"#, Some(""))]
#[case::number_key(r#"{"api_key":7}"#, None)]
#[case::null_key(r#"{"api_key":null}"#, None)]
#[case::absent_key("{}", None)]
fn a_string_stored_api_key_fills_a_missing_api_key(
    #[case] params: &str,
    #[case] expected: Option<&str>,
) {
    let lookup = FakeLookup::new(Some(agent("{}", params)));
    let mut optional_params = Object::new();
    let input = RegistryInput {
        agent_name: "agent",
        api_base: None,
        api_key: None,
        headers: None,
    };
    let resolved = resolve(&input, &mut optional_params, &lookup).unwrap();
    assert_eq!(api_key_of(&resolved), expected);
}

#[test]
fn an_explicit_api_key_is_kept() {
    let lookup = FakeLookup::new(Some(agent("{}", r#"{"api_key":"stored"}"#)));
    let key = secret("caller-key");
    let mut optional_params = Object::new();
    let input = RegistryInput {
        agent_name: "agent",
        api_base: None,
        api_key: Some(&key),
        headers: None,
    };
    let resolved = resolve(&input, &mut optional_params, &lookup).unwrap();
    assert_eq!(api_key_of(&resolved), Some("caller-key"));
}

#[test]
fn stored_headers_fill_empty_caller_headers() {
    let lookup = FakeLookup::new(Some(agent("{}", r#"{"headers":{"X":"1"}}"#)));
    let mut optional_params = Object::new();
    let input = RegistryInput {
        agent_name: "agent",
        api_base: None,
        api_key: None,
        headers: None,
    };
    let resolved = resolve(&input, &mut optional_params, &lookup).unwrap();
    assert_eq!(resolved.headers, Some(header_map(&[("X", "1")])));
}

#[test]
fn caller_headers_win_over_stored_headers() {
    let lookup = FakeLookup::new(Some(agent("{}", r#"{"headers":{"X":"1"}}"#)));
    let caller = header_map(&[("A", "b")]);
    let mut optional_params = Object::new();
    let input = RegistryInput {
        agent_name: "agent",
        api_base: None,
        api_key: None,
        headers: Some(&caller),
    };
    let resolved = resolve(&input, &mut optional_params, &lookup).unwrap();
    assert_eq!(resolved.headers, Some(caller));
}

#[test]
fn an_empty_caller_map_survives_when_the_agent_stores_no_headers() {
    let lookup = FakeLookup::new(Some(agent("{}", "{}")));
    let empty = BTreeMap::new();
    let mut optional_params = Object::new();
    let input = RegistryInput {
        agent_name: "agent",
        api_base: None,
        api_key: None,
        headers: Some(&empty),
    };
    let resolved = resolve(&input, &mut optional_params, &lookup).unwrap();
    assert_eq!(resolved.headers, Some(BTreeMap::new()));
}

#[rstest]
#[case::text(r#"{"headers":"x"}"#)]
#[case::null(r#"{"headers":null}"#)]
#[case::list(r#"{"headers":[]}"#)]
fn a_non_object_stored_headers_value_is_ignored(#[case] params: &str) {
    let lookup = FakeLookup::new(Some(agent("{}", params)));
    let mut optional_params = Object::new();
    let input = RegistryInput {
        agent_name: "agent",
        api_base: None,
        api_key: None,
        headers: None,
    };
    let resolved = resolve(&input, &mut optional_params, &lookup).unwrap();
    assert_eq!(resolved.headers, None);
}

#[test]
fn a_non_string_stored_header_value_names_it() {
    let lookup = FakeLookup::new(Some(agent("{}", r#"{"headers":{"X":7}}"#)));
    let mut optional_params = Object::new();
    let input = RegistryInput {
        agent_name: "agent",
        api_base: None,
        api_key: None,
        headers: None,
    };
    let error = resolve(&input, &mut optional_params, &lookup).unwrap_err();
    assert!(matches!(error, A2aFailure::HeaderType { name } if name == "X"));
}

#[test]
fn an_entra_agent_takes_the_key_from_the_resolver() {
    let lookup = FakeLookup::new(Some(agent("{}", r#"{"azure_ad_token":"entra-token"}"#)));
    let mut optional_params = Object::new();
    let input = RegistryInput {
        agent_name: "agent",
        api_base: None,
        api_key: None,
        headers: None,
    };
    let resolved = resolve(&input, &mut optional_params, &lookup).unwrap();
    assert_eq!(api_key_of(&resolved), Some("entra-token"));
}

#[rstest]
#[case::upper("Authorization")]
#[case::lower("authorization")]
#[case::mixed("AuThOrIzAtIoN")]
fn an_entra_agent_drops_the_stored_authorization_header(#[case] name: &str) {
    let params = format!(r#"{{"azure_ad_token":"t","headers":{{"{name}":"gone","X":"1"}}}}"#);
    let lookup = FakeLookup::new(Some(agent("{}", &params)));
    let mut optional_params = Object::new();
    let input = RegistryInput {
        agent_name: "agent",
        api_base: None,
        api_key: None,
        headers: None,
    };
    let resolved = resolve(&input, &mut optional_params, &lookup).unwrap();
    assert_eq!(resolved.headers, Some(header_map(&[("X", "1")])));
}

#[test]
fn a_non_entra_agent_keeps_its_stored_authorization_header() {
    let lookup = FakeLookup::new(Some(agent(
        "{}",
        r#"{"api_key":"k","headers":{"authorization":"kept"}}"#,
    )));
    let mut optional_params = Object::new();
    let input = RegistryInput {
        agent_name: "agent",
        api_base: None,
        api_key: None,
        headers: None,
    };
    let resolved = resolve(&input, &mut optional_params, &lookup).unwrap();
    assert_eq!(
        resolved.headers,
        Some(header_map(&[("authorization", "kept")]))
    );
}

#[test]
fn a_non_string_custom_llm_provider_names_the_field() {
    let lookup = FakeLookup::new(Some(agent("{}", r#"{"custom_llm_provider":7}"#)));
    let mut optional_params = Object::new();
    let input = RegistryInput {
        agent_name: "agent",
        api_base: None,
        api_key: None,
        headers: None,
    };
    let error = resolve(&input, &mut optional_params, &lookup).unwrap_err();
    assert!(matches!(
        error,
        A2aFailure::OptionType {
            field: "custom_llm_provider"
        }
    ));
}

#[test]
fn a_non_string_card_url_names_the_field() {
    let lookup = FakeLookup::new(Some(agent(r#"{"url":7}"#, "{}")));
    let mut optional_params = Object::new();
    let input = RegistryInput {
        agent_name: "agent",
        api_base: None,
        api_key: None,
        headers: None,
    };
    let error = resolve(&input, &mut optional_params, &lookup).unwrap_err();
    assert!(matches!(error, A2aFailure::CardUrlType));
}

#[test]
fn an_entra_agent_excludes_every_entra_key_from_the_merge() {
    let params = r#"{"azure_ad_token":"t","tenant_id":"tid","client_id":"cid","client_secret":"s",
        "azure_username":"u","azure_password":"p","azure_scope":"scope","timeout":7}"#;
    let lookup = FakeLookup::new(Some(agent("{}", params)));
    let mut optional_params = Object::new();
    let input = RegistryInput {
        agent_name: "agent",
        api_base: None,
        api_key: None,
        headers: None,
    };
    let resolved = resolve(&input, &mut optional_params, &lookup).unwrap();
    assert_eq!(api_key_of(&resolved), Some("t"));
    assert_eq!(optional_params, object(r#"{"timeout":7}"#));
}

#[test]
fn the_other_agent_params_merge_without_overwriting_optional_params() {
    let params =
        r#"{"timeout":7,"max_retries":2,"model":"x","api_base":"http://stored","headers":{}}"#;
    let lookup = FakeLookup::new(Some(agent("{}", params)));
    let key = secret("caller-key");
    let caller = header_map(&[("A", "b")]);
    let mut optional_params = object(r#"{"timeout":99}"#);
    let input = RegistryInput {
        agent_name: "agent",
        api_base: None,
        api_key: Some(&key),
        headers: Some(&caller),
    };
    let resolved = resolve(&input, &mut optional_params, &lookup).unwrap();
    assert_eq!(optional_params, object(r#"{"timeout":99,"max_retries":2}"#));
    assert_eq!(resolved.headers, Some(caller));
}

#[rstest]
#[case::card_without_streaming(
    r#"{"stream":true}"#,
    r#"{"capabilities":{"streaming":false}}"#,
    r#"{"stream":false,"fake_stream":true}"#
)]
#[case::empty_capabilities(
    r#"{"stream":true}"#,
    r#"{"capabilities":{}}"#,
    r#"{"stream":false,"fake_stream":true}"#
)]
#[case::no_capabilities(r#"{"stream":true}"#, "{}", r#"{"stream":true}"#)]
#[case::streaming_card(
    r#"{"stream":true}"#,
    r#"{"capabilities":{"streaming":true}}"#,
    r#"{"stream":true}"#
)]
#[case::not_streaming(
    r#"{"stream":false}"#,
    r#"{"capabilities":{"streaming":false}}"#,
    r#"{"stream":false}"#
)]
#[case::no_stream_option("{}", r#"{"capabilities":{"streaming":false}}"#, "{}")]
#[case::non_object_capabilities(
    r#"{"stream":true}"#,
    r#"{"capabilities":"yes"}"#,
    r#"{"stream":true}"#
)]
#[case::truthy_streaming(
    r#"{"stream":true}"#,
    r#"{"capabilities":{"streaming":1}}"#,
    r#"{"stream":true}"#
)]
fn the_fake_stream_pair_follows_the_card(
    #[case] optional: &str,
    #[case] card: &str,
    #[case] expected: &str,
) {
    let lookup = FakeLookup::new(Some(agent(card, "{}")));
    let mut optional_params = object(optional);
    let input = RegistryInput {
        agent_name: "agent",
        api_base: None,
        api_key: None,
        headers: None,
    };
    resolve(&input, &mut optional_params, &lookup).unwrap();
    assert_eq!(optional_params, object(expected));
}

#[test]
fn a_non_bool_registry_stream_option_names_the_field() {
    let lookup = FakeLookup::new(Some(agent("{}", "{}")));
    let mut optional_params = object(r#"{"stream":"yes"}"#);
    let input = RegistryInput {
        agent_name: "agent",
        api_base: None,
        api_key: None,
        headers: None,
    };
    let error = resolve(&input, &mut optional_params, &lookup).unwrap_err();
    assert!(matches!(error, A2aFailure::OptionType { field: "stream" }));
}

#[test]
fn a_failing_entra_resolver_carries_its_error() {
    let lookup = FakeLookup::new(Some(agent("{}", r#"{"azure_ad_token":"t"}"#)));
    let mut optional_params = Object::new();
    let input = RegistryInput {
        agent_name: "agent",
        api_base: None,
        api_key: None,
        headers: None,
    };
    let error = resolve_agent_config(&input, &mut optional_params, Some(&lookup), &RefusingEntra)
        .unwrap_err();
    assert!(matches!(error, A2aFailure::Credentials(_)));
}

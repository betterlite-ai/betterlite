#![cfg(test)]

use std::fmt::Debug;
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use betterlite_config::sections::jwt_settings::records::{RoutingOverride, Selector};
use rstest::rstest;
use serde_json::json;
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::layer::{Context, Layer};

use super::{
    matches_routing_override, routing_selector_matches_claim, should_route_jwt_to_oauth2_override,
};
use crate::jwt_claims::TypedValue;

const MATCHED_MESSAGE: &str =
    "JWT routing override matched. Routing token to OAuth2 introspection.";

#[derive(Clone, Debug, PartialEq, Eq)]
struct Logged {
    level: Level,
    message: String,
}

struct MessageVisitor(String);

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

#[derive(Clone, Default)]
struct Recorder(Arc<Mutex<Vec<Logged>>>);

impl<S: Subscriber> Layer<S> for Recorder {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let mut visitor = MessageVisitor(String::new());
        event.record(&mut visitor);
        let logged = Logged {
            level: *event.metadata().level(),
            message: visitor.0,
        };
        self.0.lock().unwrap().push(logged);
    }
}

fn observed<T, F: FnOnce() -> T>(run: F) -> (T, Vec<Logged>) {
    let recorder = Recorder::default();
    let subscriber = tracing_subscriber::registry().with(recorder.clone());
    let value = with_default(subscriber, run);
    let events = recorder.0.lock().unwrap().clone();
    (value, events)
}

fn document(raw: &str) -> TypedValue {
    serde_json::from_str(raw).unwrap()
}

fn one(selector: &str) -> Option<Selector> {
    Some(Selector::One(selector.to_owned()))
}

fn many(selectors: &[&str]) -> Option<Selector> {
    let values = selectors.iter().map(|value| String::from(*value)).collect();
    Some(Selector::Many(values))
}

fn rule(raw: &str) -> RoutingOverride {
    serde_json::from_str(raw).unwrap()
}

const HEADER: &str = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9";

fn signed_token(claims: &str) -> String {
    let payload = URL_SAFE_NO_PAD.encode(claims);
    format!("{HEADER}.{payload}.c2ln")
}

fn issuer_token() -> String {
    signed_token(r#"{"iss": "https://issuer.example", "aud": "api"}"#)
}

fn messages(events: &[Logged]) -> Vec<&str> {
    events.iter().map(|event| event.message.as_str()).collect()
}

#[rstest]
#[case::null_selector_matches_absent_claim(None, "{}", true)]
#[case::null_selector_matches_null_claim(None, r#"{"iss": null}"#, true)]
#[case::absent_claim_never_matches(one("issuer"), "{}", false)]
#[case::null_claim_never_matches(one("issuer"), r#"{"iss": null}"#, false)]
#[case::empty_selector_list_never_matches(many(&[]), r#"{"iss": "issuer"}"#, false)]
#[case::empty_claim_list_never_matches(one("issuer"), r#"{"iss": []}"#, false)]
#[case::empty_selector_and_claim_lists_never_match(many(&[]), r#"{"iss": []}"#, false)]
#[case::single_selector_matches_string_claim(one("issuer"), r#"{"iss": "issuer"}"#, true)]
#[case::single_selector_misses_another_claim(one("issuer"), r#"{"iss": "other"}"#, false)]
#[case::empty_selector_matches_empty_claim(one(""), r#"{"iss": ""}"#, true)]
#[case::empty_selector_misses_nonempty_claim(one(""), r#"{"iss": "a"}"#, false)]
#[case::selector_list_matches_claim_list(many(&["a", "b"]), r#"{"iss": ["b"]}"#, true)]
fn selector_null_absent_and_empty_lists(
    #[case] selector: Option<Selector>,
    #[case] claims: &str,
    #[case] expected: bool,
) {
    let found = routing_selector_matches_claim(selector.as_ref(), &document(claims), "iss", false);
    assert_eq!(found, expected);
}

#[rstest]
#[case::space_splits_scope("openid profile", one("profile"), true, true)]
#[case::unsplit_scope_keeps_the_whole_string("openid profile", one("profile"), false, false)]
#[case::unsplit_scope_matches_its_full_text("openid profile", one("openid profile"), false, true)]
#[case::split_scope_matches_no_multiword_selector(
    "openid profile",
    one("openid profile"),
    true,
    false
)]
#[case::padded_scope_keeps_its_padding("  openid  ", one("openid"), true, false)]
#[case::padded_scope_matches_the_padded_selector("  openid  ", one("  openid  "), true, true)]
#[case::split_drops_the_empty_pieces(" a  b ", one("b"), true, true)]
#[case::split_space_never_equals_one_token(" a  b ", one("a b"), true, false)]
#[case::tab_never_splits("a\tb", one("a"), true, false)]
#[case::tab_keeps_the_exact_text("a\tb", one("a\tb"), true, true)]
#[case::nonbreaking_space_never_splits("a\u{a0}b", one("a"), true, false)]
#[case::whitespace_only_scope_is_kept(" ", one(" "), true, true)]
#[case::whitespace_only_scope_is_not_empty(" ", one(""), true, false)]
#[case::separator_before_the_split_is_stripped("\u{1c}a b", one("b"), true, true)]
#[case::separator_inside_a_token_never_splits("a\u{1c}b", one("a"), true, false)]
#[case::single_token_scope_matches("openid", one("openid"), true, true)]
fn scope_splits_on_ascii_space_only(
    #[case] claim: &str,
    #[case] selector: Option<Selector>,
    #[case] split_space_delimited: bool,
    #[case] expected: bool,
) {
    let claims = TypedValue {
        json: json!({ "scope": claim }),
    };
    let found =
        routing_selector_matches_claim(selector.as_ref(), &claims, "scope", split_space_delimited);
    assert_eq!(found, expected);
}

#[test]
fn scope_list_claims_never_split() {
    let claims = document(r#"{"scope": ["openid profile"]}"#);
    assert!(!routing_selector_matches_claim(
        one("profile").as_ref(),
        &claims,
        "scope",
        true
    ));
    assert!(routing_selector_matches_claim(
        one("openid profile").as_ref(),
        &claims,
        "scope",
        true
    ));
}

#[rstest]
#[case::star_matches_the_prefix("trusted.*", "trusted.example.com", false, true)]
#[case::star_refuses_a_space("trusted.*", "trusted.example.com evil.com", false, false)]
#[case::star_refuses_a_unit_separator(
    "trusted.*",
    "trusted.example.com\u{1c}evil.com",
    false,
    false
)]
#[case::star_refuses_a_tab("trusted.*", "trusted.example.com\tevil.com", false, false)]
#[case::star_refuses_a_newline("trusted.*", "trusted.example.com\nevil.com", false, false)]
#[case::star_refuses_a_nonbreaking_space(
    "trusted.*",
    "trusted.example.com\u{a0}evil.com",
    false,
    false
)]
#[case::scope_star_spans_a_tab("App:*", "App:\tLiteLLM", true, true)]
#[case::wildcards_are_case_sensitive("TRUSTED.*", "trusted.example.com", false, false)]
#[case::question_matches_one_letter("a?c", "abc", false, true)]
#[case::question_needs_one_letter("a?c", "ac", false, false)]
#[case::question_refuses_a_space("a?c", "a c", false, false)]
#[case::question_refuses_a_unit_separator("a?c", "a\u{1c}c", false, false)]
#[case::brackets_alone_are_literal("[ab]c", "[ab]c", false, true)]
#[case::brackets_alone_never_match_a_class("[ab]c", "ac", false, false)]
#[case::brackets_with_a_star_are_a_pattern("[ab]*", "acme", false, true)]
#[case::question_inside_brackets_is_a_pattern("[?]", "?", false, true)]
#[case::an_exact_selector_may_hold_spaces("a b", "a b", false, true)]
#[case::a_bare_star_matches_anything("*", "trusted.example.com", false, true)]
#[case::a_trailing_star_refuses_a_space("a*", "a b", false, false)]
fn wildcard_case_whitespace_and_brackets(
    #[case] selector: &str,
    #[case] claim: &str,
    #[case] split_space_delimited: bool,
    #[case] expected: bool,
) {
    let claims = TypedValue {
        json: json!({ "iss": claim }),
    };
    let found = routing_selector_matches_claim(
        one(selector).as_ref(),
        &claims,
        "iss",
        split_space_delimited,
    );
    assert_eq!(found, expected);
}

#[rstest]
#[case::second_selector_second_claim(many(&["x", "y"]), r#"{"iss": ["a", "y"]}"#, true)]
#[case::second_selector_first_claim(many(&["x", "y"]), r#"{"iss": ["y", "a"]}"#, true)]
#[case::no_pair_matches(many(&["x", "z"]), r#"{"iss": ["a", "b"]}"#, false)]
#[case::one_selector_matches_the_last_claim(one("b"), r#"{"iss": ["a", "b"]}"#, true)]
#[case::a_star_selector_matches_any_claim(one("*"), r#"{"iss": ["a", "b"]}"#, true)]
#[case::two_patterns_share_one_claim(many(&["a*", "*c"]), r#"{"iss": ["abc"]}"#, true)]
#[case::an_empty_element_is_a_value(one(""), r#"{"iss": ["", "x"]}"#, true)]
#[case::an_empty_selector_matches_the_empty_element(many(&[""]), r#"{"iss": [""]}"#, true)]
#[case::an_empty_claim_list_has_no_value(many(&["a"]), r#"{"iss": []}"#, false)]
fn selector_and_claim_cross_product(
    #[case] selector: Option<Selector>,
    #[case] claims: &str,
    #[case] expected: bool,
) {
    let found = routing_selector_matches_claim(selector.as_ref(), &document(claims), "iss", false);
    assert_eq!(found, expected);
}

#[rstest]
#[case::number_claim(r#"{"iss": 5}"#, one("5"), false)]
#[case::float_claim(r#"{"iss": 1.5}"#, one("1.5"), false)]
#[case::bool_claim(r#"{"iss": true}"#, one("true"), false)]
#[case::object_claim(r#"{"iss": {"a": 1}}"#, one(r#"{"a": 1}"#), false)]
#[case::number_in_a_list(r#"{"iss": [5]}"#, one("5"), false)]
#[case::other_types_in_a_list(r#"{"iss": [true, {"a": 1}]}"#, one("true"), false)]
#[case::string_members_still_count(r#"{"iss": ["admin", 5]}"#, one("admin"), true)]
#[case::string_members_match_patterns(r#"{"iss": [7, "admin"]}"#, one("adm*"), true)]
#[case::null_members_are_skipped(r#"{"iss": [null, "admin"]}"#, one("admin"), true)]
#[case::nested_lists_have_no_value(r#"{"iss": [["a"]]}"#, one("a"), false)]
#[case::only_non_strings_never_match_a_star(r#"{"iss": [5, true]}"#, one("*"), false)]
#[case::an_empty_object_matches_no_empty_selector(r#"{"iss": {}}"#, one(""), false)]
fn non_string_claims_never_match(
    #[case] claims: &str,
    #[case] selector: Option<Selector>,
    #[case] expected: bool,
) {
    let found = routing_selector_matches_claim(selector.as_ref(), &document(claims), "iss", false);
    assert_eq!(found, expected);
}

#[rstest]
#[case::all_four_match(
    r#"{"iss": "https://idp", "client_id": "client-abc", "scope": "openid", "aud": "api"}"#,
    r#"{"iss": "https://idp", "client_id": "client-abc", "scope": "openid profile", "aud": "api"}"#,
    true
)]
#[case::iss_mismatch(
    r#"{"iss": "https://idp", "client_id": "client-abc", "scope": "openid", "aud": "api"}"#,
    r#"{"iss": "https://other", "client_id": "client-abc", "scope": "openid", "aud": "api"}"#,
    false
)]
#[case::client_id_mismatch(
    r#"{"iss": "https://idp", "client_id": "client-abc", "scope": "openid", "aud": "api"}"#,
    r#"{"iss": "https://idp", "client_id": "client-other", "scope": "openid", "aud": "api"}"#,
    false
)]
#[case::scope_mismatch(
    r#"{"iss": "https://idp", "client_id": "client-abc", "scope": "openid", "aud": "api"}"#,
    r#"{"iss": "https://idp", "client_id": "client-abc", "scope": "email", "aud": "api"}"#,
    false
)]
#[case::aud_mismatch(
    r#"{"iss": "https://idp", "client_id": "client-abc", "scope": "openid", "aud": "api"}"#,
    r#"{"iss": "https://idp", "client_id": "client-abc", "scope": "openid", "aud": "other"}"#,
    false
)]
#[case::omitted_client_id_matches_any(
    r#"{"iss": "https://idp"}"#,
    r#"{"iss": "https://idp", "client_id": "client-abc"}"#,
    true
)]
#[case::absent_claims_fail_the_iss_selector(r#"{"iss": "https://idp"}"#, "{}", false)]
#[case::aud_list_matches_a_list_claim(
    r#"{"iss": "https://idp", "aud": ["api", "other"]}"#,
    r#"{"iss": "https://idp", "aud": ["other"]}"#,
    true
)]
#[case::iss_list_matches_a_string_claim(
    r#"{"iss": ["https://x", "https://idp"]}"#,
    r#"{"iss": "https://idp"}"#,
    true
)]
#[case::scope_wildcard_checks_each_token(
    r#"{"iss": "https://idp", "scope": "App:*"}"#,
    r#"{"iss": "https://idp", "scope": "App:LiteLLM App:API"}"#,
    true
)]
#[case::aud_wildcard_refuses_spaces(
    r#"{"iss": "https://idp", "aud": "api*"}"#,
    r#"{"iss": "https://idp", "aud": "api v2"}"#,
    false
)]
#[case::scope_selector_never_keeps_a_space(
    r#"{"iss": "https://idp", "scope": "openid profile"}"#,
    r#"{"iss": "https://idp", "scope": "openid profile"}"#,
    false
)]
#[case::aud_selector_keeps_a_space(
    r#"{"iss": "https://idp", "aud": "api v2"}"#,
    r#"{"iss": "https://idp", "aud": "api v2"}"#,
    true
)]
#[case::client_id_wildcard_matches(
    r#"{"iss": "https://idp", "client_id": "client-*"}"#,
    r#"{"iss": "https://idp", "client_id": "client-abc"}"#,
    true
)]
fn override_requires_every_selector(
    #[case] override_rule: &str,
    #[case] claims: &str,
    #[case] expected: bool,
) {
    let found = matches_routing_override(&document(claims), &rule(override_rule));
    assert_eq!(found, expected);
}

#[test]
fn route_takes_first_oauth2_match() {
    let token = issuer_token();
    let mixed = vec![
        rule(r#"{"iss": "https://other.example"}"#),
        rule(r#"{"iss": "https://issuer.example"}"#),
    ];
    let (routed, events) =
        observed(|| should_route_jwt_to_oauth2_override(&token, Some(mixed.as_slice())));
    assert!(routed);
    assert_eq!(messages(&events), vec![MATCHED_MESSAGE]);
    let both = vec![
        rule(r#"{"iss": "https://issuer.example"}"#),
        rule(r#"{"iss": "https://issuer.example", "aud": "api"}"#),
    ];
    let (again, logs) =
        observed(|| should_route_jwt_to_oauth2_override(&token, Some(both.as_slice())));
    assert!(again);
    assert_eq!(messages(&logs), vec![MATCHED_MESSAGE]);
}

#[test]
fn route_false_without_overrides_or_claims() {
    let token = issuer_token();
    assert!(!should_route_jwt_to_oauth2_override(&token, None));
    assert!(!should_route_jwt_to_oauth2_override(&token, Some(&[])));
    let rules = vec![rule(r#"{"iss": "https://issuer.example"}"#)];
    assert!(!should_route_jwt_to_oauth2_override(
        "not-a-jwt",
        Some(rules.as_slice())
    ));
    assert!(!should_route_jwt_to_oauth2_override(
        "e30.bm90LWpzb24.c2ln",
        Some(rules.as_slice())
    ));
    assert!(!should_route_jwt_to_oauth2_override(
        "e30.WzEsMl0.c2ln",
        Some(rules.as_slice())
    ));
    assert!(!should_route_jwt_to_oauth2_override(
        "e30.e30.c2ln",
        Some(rules.as_slice())
    ));
    assert!(!should_route_jwt_to_oauth2_override(
        &signed_token(r#"{"iss": "https://other.example"}"#),
        Some(rules.as_slice())
    ));
    assert!(should_route_jwt_to_oauth2_override(
        &token,
        Some(rules.as_slice())
    ));
}

#[test]
fn route_false_when_a_segment_is_not_base64() {
    let payload = URL_SAFE_NO_PAD.encode(r#"{"iss": "https://issuer.example"}"#);
    let rules = vec![rule(r#"{"iss": "https://issuer.example"}"#)];
    assert!(!should_route_jwt_to_oauth2_override(
        &format!("{HEADER}.{payload}.!!!"),
        Some(rules.as_slice())
    ));
    assert!(!should_route_jwt_to_oauth2_override(
        &format!("!!!.{payload}.c2ln"),
        Some(rules.as_slice())
    ));
    assert!(should_route_jwt_to_oauth2_override(
        &format!("{HEADER}.{payload}.c2ln"),
        Some(rules.as_slice())
    ));
}

#[test]
fn match_log_holds_no_claim_or_token() {
    let claims = r#"{"iss": "https://issuer.example/top-secret-issuer", "aud": "top-secret-audience", "client_id": "top-secret-client", "scope": "top-secret-scope openid"}"#;
    let token = signed_token(claims);
    let rules = vec![rule(
        r#"{"iss": "https://issuer.example/top-secret-issuer", "aud": "top-secret-audience", "client_id": "top-secret-client", "scope": "top-secret-scope"}"#,
    )];
    let (routed, events) =
        observed(|| should_route_jwt_to_oauth2_override(&token, Some(rules.as_slice())));
    assert!(routed);
    let levels: Vec<Level> = events.iter().map(|event| event.level).collect();
    assert_eq!(levels, vec![Level::DEBUG]);
    assert_eq!(messages(&events), vec![MATCHED_MESSAGE]);
    let secrets = [
        token.as_str(),
        "https://issuer.example/top-secret-issuer",
        "top-secret-audience",
        "top-secret-client",
        "top-secret-scope",
    ];
    let leaked: Vec<&str> = events
        .iter()
        .filter(|event| secrets.iter().any(|secret| event.message.contains(*secret)))
        .map(|event| event.message.as_str())
        .collect();
    assert!(leaked.is_empty(), "{leaked:?}");
}

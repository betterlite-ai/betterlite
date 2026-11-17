#![cfg(test)]

use std::collections::BTreeMap;

use indexmap::IndexMap;
use secrecy::ExposeSecret as _;

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::straiker::delivery::fixtures::{
    Fakes, V1_ALLOW, V3_ALLOW, delivery, inputs, raw, reply, request_data, run, sent, settings,
    v3_raw,
};
use crate::straiker::delivery::transport::compose;

fn custom(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn a_v1_post_carries_the_webhook_format() {
    let headers = compose(&settings(&raw()), &IndexMap::new());
    assert_eq!(headers.authorization.expose_secret(), "Bearer sk_live_test");
    let expected = BTreeMap::from([
        ("Content-Type".to_owned(), "application/json".to_owned()),
        ("X-Straiker-Webhook-Format".to_owned(), "litellm".to_owned()),
    ]);
    assert_eq!(headers.values, expected);
}

#[test]
fn a_v3_post_omits_the_webhook_format() {
    let headers = compose(&settings(&v3_raw()), &IndexMap::new());
    let expected = BTreeMap::from([("Content-Type".to_owned(), "application/json".to_owned())]);
    assert_eq!(headers.values, expected);
}

#[test]
fn custom_headers_are_added() {
    let mut args = raw();
    args.custom_headers = Some(custom(&[("X-Org", "acme")]));
    let headers = compose(&settings(&args), &IndexMap::new());
    assert_eq!(
        headers.values.get("X-Org").map(String::as_str),
        Some("acme")
    );
}

#[test]
fn reserved_headers_are_filtered_from_the_custom_set() {
    let mut args = raw();
    args.custom_headers = Some(custom(&[
        ("authorization", "Bearer evil"),
        ("Content-Type", "text/plain"),
        ("X-STRAIKER-WEBHOOK-FORMAT", "none"),
        ("x-custom", "kept"),
    ]));
    let headers = compose(&settings(&args), &IndexMap::new());
    assert_eq!(
        headers.values.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(
        headers
            .values
            .get("X-Straiker-Webhook-Format")
            .map(String::as_str),
        Some("litellm")
    );
    assert_eq!(
        headers.values.get("x-custom").map(String::as_str),
        Some("kept")
    );
    assert!(
        !headers
            .values
            .keys()
            .any(|name| name.eq_ignore_ascii_case("authorization"))
    );
    assert_eq!(headers.authorization.expose_secret(), "Bearer sk_live_test");
}

#[test]
fn the_composition_headers_win_over_a_custom_header() {
    let mut args = v3_raw();
    args.custom_headers = Some(custom(&[("x-s6r-agent", "from-custom")]));
    let extra = IndexMap::from([("x-s6r-agent", "from-route".to_owned())]);
    let headers = compose(&settings(&args), &extra);
    assert_eq!(
        headers.values.get("x-s6r-agent").map(String::as_str),
        Some("from-route")
    );
}

#[tokio::test]
async fn a_v1_delivery_sends_the_base_headers() {
    let fakes = Fakes::default();
    let handler = delivery(settings(&raw()), vec![reply(200, V1_ALLOW)]);
    let mut data = request_data(r#"{"model":"gpt-4o"}"#);
    let call = fakes.call(Some("completion"), None);
    run(
        &handler,
        &mut data,
        inputs(&["hi"]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("an allowed call");
    let posts = sent(&handler);
    let headers = &posts.first().unwrap().headers;
    assert_eq!(
        headers.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(
        headers.get("X-Straiker-Webhook-Format").map(String::as_str),
        Some("litellm")
    );
    assert_eq!(posts.first().unwrap().authorization, "Bearer sk_live_test");
}

#[tokio::test]
async fn a_v3_delivery_sends_the_composition_headers() {
    let mut args = v3_raw();
    args.agent_ref = Some("agent-7".to_owned());
    args.client = Some("claude".to_owned());
    args.format_hint = Some("openai.chat".to_owned());
    let fakes = Fakes::default();
    let handler = delivery(settings(&args), vec![reply(200, V3_ALLOW)]);
    let mut data =
        request_data(r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hello"}]}"#);
    let call = fakes.call(Some("completion"), None);
    run(
        &handler,
        &mut data,
        inputs(&[]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("an allowed call");
    let posts = sent(&handler);
    let headers = &posts.first().unwrap().headers;
    assert_eq!(
        headers.get("x-s6r-agent").map(String::as_str),
        Some("agent-7")
    );
    assert_eq!(
        headers.get("x-s6r-client").map(String::as_str),
        Some("claude")
    );
    assert_eq!(
        headers.get("x-s6r-format").map(String::as_str),
        Some("openai.chat")
    );
    assert!(!headers.contains_key("X-Straiker-Webhook-Format"));
}

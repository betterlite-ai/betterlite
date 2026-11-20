use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::{
    Reply, Scripted, guardrail, guardrail_with_budget, input_scan, masked, message, model_response,
    output_scan, passing, run, too_large,
};
use crate::guardrail_base::failure::GuardrailFailure;

fn item(text: &str) -> Value {
    json!({ "text": { "text": text } })
}

#[tokio::test]
async fn a_multi_item_payload_is_bin_packed_into_batches() {
    let messages = vec![
        message("user", json!("aaaa")),
        message("user", json!("bbbb")),
        message("user", json!("cccc")),
    ];
    let transport = Scripted::new(vec![too_large(), passing(5), passing(7)]);
    let mut data = BTreeMap::new();
    let outcome = run(
        &guardrail_with_budget(8),
        &mut data,
        input_scan(&messages),
        &transport,
    )
    .await;
    drop(outcome.expect("both batches pass"));
    assert_eq!(transport.count(), 3);
    let bodies = transport.bodies();
    assert_eq!(
        bodies[0]["content"],
        json!([item("aaaa"), item("bbbb"), item("cccc")])
    );
    assert_eq!(bodies[1]["content"], json!([item("aaaa"), item("bbbb")]));
    assert_eq!(bodies[2]["content"], json!([item("cccc")]));
}

#[tokio::test]
async fn a_single_batch_of_items_falls_back_to_bisection() {
    let messages = vec![
        message("user", json!("aaaa")),
        message("user", json!("bbbb")),
    ];
    let transport = Scripted::new(vec![too_large(), passing(5), passing(7)]);
    let mut data = BTreeMap::new();
    let outcome = run(
        &guardrail_with_budget(100),
        &mut data,
        input_scan(&messages),
        &transport,
    )
    .await;
    drop(outcome.expect("both halves pass"));
    assert_eq!(transport.count(), 3);
    let bodies = transport.bodies();
    assert_eq!(bodies[1]["content"], json!([item("aaaa")]));
    assert_eq!(bodies[2]["content"], json!([item("bbbb")]));
}

#[tokio::test]
async fn a_single_oversized_text_is_bisected_and_rejoined() {
    let messages = vec![message("user", json!("aaaa bbbb"))];
    let transport = Scripted::new(vec![too_large(), masked("aaaa "), masked("bbbb")]);
    let mut data = BTreeMap::new();
    let outcome = run(
        &guardrail_with_budget(100),
        &mut data,
        input_scan(&messages),
        &transport,
    )
    .await;
    let response = outcome.expect("both halves pass");
    let wire = serde_json::to_value(&response).expect("the response encodes");
    assert_eq!(wire["outputs"], json!([{ "text": "aaaa bbbb" }]));
    assert_eq!(wire["output"], json!([{ "text": "aaaa bbbb" }]));
    assert_eq!(transport.count(), 3);
}

#[tokio::test]
async fn a_repeated_bisection_stamps_the_whole_group() {
    let messages = vec![message("user", json!("aaaa bbbb cccc dddd"))];
    let transport = Scripted::new(vec![
        too_large(),
        too_large(),
        masked("aaaa "),
        masked("bbbb "),
        masked("cccc dddd"),
    ]);
    let mut data = BTreeMap::new();
    let outcome = run(
        &guardrail_with_budget(100),
        &mut data,
        input_scan(&messages),
        &transport,
    )
    .await;
    let response = outcome.expect("every leaf passes");
    let wire = serde_json::to_value(&response).expect("the response encodes");
    assert_eq!(wire["outputs"], json!([{ "text": "aaaa bbbb cccc dddd" }]));
    assert_eq!(transport.count(), 5);
}

#[tokio::test]
async fn a_grounded_payload_is_never_chunked() {
    let messages = vec![message(
        "user",
        json!([{ "type": "query", "text": "what is the policy?" }]),
    )];
    let response = model_response("the policy allows it");
    let transport = Scripted::new(vec![too_large()]);
    let mut data = BTreeMap::new();
    let outcome = run(
        &guardrail(),
        &mut data,
        output_scan(Some(&messages), &response),
        &transport,
    )
    .await;
    let failure = outcome.expect_err("grounding is not split");
    let GuardrailFailure::Http { status, .. } = failure else {
        panic!("a too-large reply is a plain http failure");
    };
    assert_eq!(status, 429);
    assert_eq!(transport.count(), 1);
    let bodies = transport.bodies();
    assert_eq!(bodies[0]["source"], json!("OUTPUT"));
    assert_eq!(
        bodies[0]["content"],
        json!([
            { "text": { "text": "what is the policy?", "qualifiers": ["query"] } },
            { "text": { "text": "the policy allows it", "qualifiers": ["guard_content"] } },
        ])
    );
}

#[tokio::test]
async fn an_unsplittable_too_large_item_is_not_retried_or_split() {
    let messages = vec![message("user", json!("a"))];
    let transport = Scripted::new(vec![too_large(), Reply::ok(json!({ "action": "NONE" }))]);
    let mut data = BTreeMap::new();
    let outcome = run(
        &guardrail_with_budget(100),
        &mut data,
        input_scan(&messages),
        &transport,
    )
    .await;
    let failure = outcome.expect_err("a one-character item cannot be split");
    let GuardrailFailure::Http { status, .. } = failure else {
        panic!("a too-large reply is a plain http failure");
    };
    assert_eq!(status, 429);
    assert_eq!(transport.count(), 1);
}

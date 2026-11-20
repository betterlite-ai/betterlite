use std::collections::BTreeMap;

use betterlite_models::decisions::Json;

use super::{
    Reply, Scripted, expected, field_of, guardrail, guardrail_with_budget, input_scan, masked,
    message, model_response, output_scan, passing, quoted, run, structured_message, text,
    too_large, wire_of,
};
use crate::guardrail_base::failure::GuardrailFailure;

fn item(value: &str) -> Json {
    expected(&format!(r#"{{"text":{{"text":{}}}}}"#, quoted(value)))
}

fn items(values: &[&str]) -> Json {
    Json::List(values.iter().copied().map(item).collect())
}

#[tokio::test]
async fn a_multi_item_payload_is_bin_packed_into_batches() {
    let messages = vec![
        message("user", "aaaa"),
        message("user", "bbbb"),
        message("user", "cccc"),
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
        field_of(&bodies[0], "content"),
        items(&["aaaa", "bbbb", "cccc"])
    );
    assert_eq!(field_of(&bodies[1], "content"), items(&["aaaa", "bbbb"]));
    assert_eq!(field_of(&bodies[2], "content"), items(&["cccc"]));
}

#[tokio::test]
async fn a_single_batch_of_items_falls_back_to_bisection() {
    let messages = vec![message("user", "aaaa"), message("user", "bbbb")];
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
    assert_eq!(field_of(&bodies[1], "content"), items(&["aaaa"]));
    assert_eq!(field_of(&bodies[2], "content"), items(&["bbbb"]));
}

#[tokio::test]
async fn a_single_oversized_text_is_bisected_and_rejoined() {
    let messages = vec![message("user", "aaaa bbbb")];
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
    let wire = wire_of(&response);
    assert_eq!(
        field_of(&wire, "outputs"),
        expected(r#"[{ "text": "aaaa bbbb" }]"#)
    );
    assert_eq!(
        field_of(&wire, "output"),
        expected(r#"[{ "text": "aaaa bbbb" }]"#)
    );
    assert_eq!(transport.count(), 3);
}

#[tokio::test]
async fn a_repeated_bisection_stamps_the_whole_group() {
    let messages = vec![message("user", "aaaa bbbb cccc dddd")];
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
    let wire = wire_of(&response);
    assert_eq!(
        field_of(&wire, "outputs"),
        expected(r#"[{ "text": "aaaa bbbb cccc dddd" }]"#)
    );
    assert_eq!(transport.count(), 5);
}

#[tokio::test]
async fn a_grounded_payload_is_never_chunked() {
    let messages = vec![structured_message(
        "user",
        r#"[{ "type": "query", "text": "what is the policy?" }]"#,
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
    assert_eq!(field_of(&bodies[0], "source"), text("OUTPUT"));
    assert_eq!(
        field_of(&bodies[0], "content"),
        expected(
            r#"[
                { "text": { "text": "what is the policy?", "qualifiers": ["query"] } },
                { "text": { "text": "the policy allows it", "qualifiers": ["guard_content"] } }
            ]"#
        )
    );
}

#[tokio::test]
async fn an_unsplittable_too_large_item_is_not_retried_or_split() {
    let messages = vec![message("user", "a")];
    let transport = Scripted::new(vec![too_large(), Reply::ok(r#"{ "action": "NONE" }"#)]);
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

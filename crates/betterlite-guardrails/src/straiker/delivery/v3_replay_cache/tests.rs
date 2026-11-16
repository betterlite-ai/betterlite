#![cfg(test)]

use std::time::{SystemTime, UNIX_EPOCH};

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::straiker::delivery::blocked_turns::system_clock;
use crate::straiker::delivery::fixtures::{
    Clock, Fakes, V3_ALLOW, V3_BLOCK, V3_BLOCK_STATE, delivery_at, inputs, reply, request_data,
    run, sent, settings, v3_raw,
};

const HELLO: &str = r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hello"}]}"#;
const ITEM_LIMIT: usize = 1024 * 1024;

fn blocked(failure: GuardrailFailure) -> String {
    let GuardrailFailure::GuardrailRaised { message, .. } = failure else {
        panic!("expected a raised block");
    };
    message
}

#[tokio::test]
async fn a_blocked_turn_is_remembered_until_its_ttl_ends() {
    let clock = Clock::default();
    let fakes = Fakes::default();
    let handler = delivery_at(
        settings(&v3_raw()),
        vec![reply(200, V3_BLOCK), reply(200, V3_ALLOW)],
        &clock,
    );
    let call = fakes.call(Some("completion"), None);
    let mut first = request_data(HELLO);
    let failure = run(
        &handler,
        &mut first,
        inputs(&[]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect_err("the turn blocks");
    assert_eq!(blocked(failure), "policy");
    clock.advance(86_400.0);
    let mut at_the_limit = request_data(HELLO);
    let replay = run(
        &handler,
        &mut at_the_limit,
        inputs(&[]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect_err("exactly one day later the block still holds");
    assert_eq!(blocked(replay), "policy");
    assert_eq!(sent(&handler).len(), 1);
}

#[tokio::test]
async fn a_blocked_turn_is_asked_again_past_its_ttl() {
    let clock = Clock::default();
    let fakes = Fakes::default();
    let handler = delivery_at(
        settings(&v3_raw()),
        vec![reply(200, V3_BLOCK), reply(200, V3_ALLOW)],
        &clock,
    );
    let call = fakes.call(Some("completion"), None);
    let mut first = request_data(HELLO);
    run(
        &handler,
        &mut first,
        inputs(&[]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect_err("the turn blocks");
    clock.advance(86_401.0);
    let mut past_the_limit = request_data(HELLO);
    run(
        &handler,
        &mut past_the_limit,
        inputs(&[]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("past its ttl the turn is asked again");
    assert_eq!(sent(&handler).len(), 2);
}

#[tokio::test]
async fn a_block_without_a_control_is_not_remembered() {
    let fakes = Fakes::default();
    let handler = delivery_at(
        settings(&v3_raw()),
        vec![reply(200, V3_BLOCK_STATE), reply(200, V3_ALLOW)],
        &Clock::default(),
    );
    let call = fakes.call(Some("completion"), None);
    let mut first = request_data(HELLO);
    let failure = run(
        &handler,
        &mut first,
        inputs(&[]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect_err("the turn blocks");
    assert_eq!(blocked(failure), "kill switch");
    let mut second = request_data(HELLO);
    run(
        &handler,
        &mut second,
        inputs(&[]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("a block with no control names no turn to remember");
    assert_eq!(sent(&handler).len(), 2);
}

#[tokio::test]
async fn an_allowed_turn_is_not_remembered() {
    let fakes = Fakes::default();
    let handler = delivery_at(
        settings(&v3_raw()),
        vec![reply(200, V3_ALLOW), reply(200, V3_ALLOW)],
        &Clock::default(),
    );
    let call = fakes.call(Some("completion"), None);
    let mut first = request_data(HELLO);
    run(
        &handler,
        &mut first,
        inputs(&[]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("the turn is allowed");
    let mut second = request_data(HELLO);
    run(
        &handler,
        &mut second,
        inputs(&[]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("the turn is allowed again");
    assert_eq!(sent(&handler).len(), 2);
}

#[tokio::test]
async fn a_block_without_a_scope_is_not_remembered() {
    let fakes = Fakes::default();
    let handler = delivery_at(
        settings(&v3_raw()),
        vec![reply(200, V3_BLOCK), reply(200, V3_ALLOW)],
        &Clock::default(),
    );
    let call = fakes.call(Some("completion"), None);
    let body = r#"{"model":"gpt-4o"}"#;
    let mut first = request_data(body);
    let failure = run(
        &handler,
        &mut first,
        inputs(&[]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect_err("the turn blocks");
    assert_eq!(blocked(failure), "policy");
    let mut second = request_data(body);
    run(
        &handler,
        &mut second,
        inputs(&[]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("there is no conversation to remember");
    assert_eq!(sent(&handler).len(), 2);
}

#[tokio::test]
async fn a_message_too_large_for_the_cache_is_not_remembered() {
    let message = "a".repeat(ITEM_LIMIT.saturating_sub(1));
    let body = format!(
        r#"{{"straiker":{{"action":"block","blocked_by":["pii"],"block_message":"{message}"}}}}"#
    );
    let fakes = Fakes::default();
    let handler = delivery_at(
        settings(&v3_raw()),
        vec![reply(200, &body), reply(200, V3_ALLOW)],
        &Clock::default(),
    );
    let call = fakes.call(Some("completion"), None);
    let mut first = request_data(HELLO);
    let failure = run(
        &handler,
        &mut first,
        inputs(&[]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect_err("the turn blocks");
    assert_eq!(blocked(failure), message);
    let mut second = request_data(HELLO);
    run(
        &handler,
        &mut second,
        inputs(&[]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("the oversized message never reached the cache");
    assert_eq!(sent(&handler).len(), 2);
}

#[tokio::test]
async fn a_message_at_the_item_limit_is_remembered() {
    let message = "a".repeat(ITEM_LIMIT.saturating_sub(2));
    let body = format!(
        r#"{{"straiker":{{"action":"block","blocked_by":["pii"],"block_message":"{message}"}}}}"#
    );
    let fakes = Fakes::default();
    let handler = delivery_at(
        settings(&v3_raw()),
        vec![reply(200, &body), reply(200, V3_ALLOW)],
        &Clock::default(),
    );
    let call = fakes.call(Some("completion"), None);
    let mut first = request_data(HELLO);
    run(
        &handler,
        &mut first,
        inputs(&[]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect_err("the turn blocks");
    let mut second = request_data(HELLO);
    let replay = run(
        &handler,
        &mut second,
        inputs(&[]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect_err("the message fit the cache");
    assert_eq!(blocked(replay), message);
    assert_eq!(sent(&handler).len(), 1);
}

#[tokio::test]
async fn a_message_of_json_null_is_not_replayed() {
    let fakes = Fakes::default();
    let handler = delivery_at(
        settings(&v3_raw()),
        vec![
            reply(
                200,
                r#"{"straiker":{"action":"block","blocked_by":["pii"],"block_message":"null"}}"#,
            ),
            reply(200, V3_ALLOW),
        ],
        &Clock::default(),
    );
    let call = fakes.call(Some("completion"), None);
    let mut first = request_data(HELLO);
    let failure = run(
        &handler,
        &mut first,
        inputs(&[]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect_err("the turn blocks");
    assert_eq!(blocked(failure), "null");
    let mut second = request_data(HELLO);
    run(
        &handler,
        &mut second,
        inputs(&[]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("the cache decoded the message as null, so nothing replays");
    assert_eq!(sent(&handler).len(), 2);
}

#[tokio::test]
async fn a_message_that_decodes_as_a_number_blocks_with_its_json_text() {
    let fakes = Fakes::default();
    let handler = delivery_at(
        settings(&v3_raw()),
        vec![
            reply(
                200,
                r#"{"straiker":{"action":"block","blocked_by":["pii"],"block_message":"123"}}"#,
            ),
            reply(200, V3_ALLOW),
        ],
        &Clock::default(),
    );
    let call = fakes.call(Some("completion"), None);
    let mut first = request_data(HELLO);
    run(
        &handler,
        &mut first,
        inputs(&[]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect_err("the turn blocks");
    let mut second = request_data(HELLO);
    let replay = run(
        &handler,
        &mut second,
        inputs(&[]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect_err("a decoded number is not null, so it replays");
    assert_eq!(blocked(replay), "123");
    assert_eq!(sent(&handler).len(), 1);
}

#[test]
fn the_system_clock_reads_wall_time_in_seconds() {
    let before = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    let now = system_clock()();
    let after = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    assert!(now >= before, "got {now}, read before {before}");
    assert!(now <= after, "got {now}, read after {after}");
}

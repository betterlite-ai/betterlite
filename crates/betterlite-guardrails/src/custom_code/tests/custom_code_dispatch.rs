use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::fake::ALLOW_SOURCE;
use super::fake::ASYNC_SOURCE;
use super::fake::FakeHost;
use super::fake::LoadReply;
use super::fake::build;
use super::fake::build_with_clock;
use super::fake::inputs;
use super::fake::never;
use super::fake::object;
use super::fake::params;
use super::fake::params_with_timeout;
use super::fake::parking_then_pending;
use super::fake::pending;
use super::fake::spinning;
use super::fake::text;
use super::fake::value;
use super::fake::wall_clock;
use crate::custom_code::error::CustomCodeApplyError;
use crate::generic_guardrail_contracts::GuardrailInputType;

fn allow_result() -> GuardrailValue {
    object(vec![("action", text("allow"))])
}

async fn apply(host: &Arc<FakeHost>, source: &str) -> Result<(), CustomCodeApplyError> {
    let guardrail = build(params(source), host);
    let mut request_data = BTreeMap::new();
    guardrail
        .apply_guardrail(
            inputs(&["hello"]),
            &mut request_data,
            GuardrailInputType::Request,
        )
        .await
        .map(|_returned| ())
}

#[tokio::test]
async fn a_sync_function_runs_off_the_event_loop_thread() {
    let host = FakeHost::new(LoadReply::Module(value(allow_result())));
    apply(&host, ALLOW_SOURCE).await.unwrap();
    let call_thread = host.log.threads.lock().unwrap().first().copied().unwrap();
    assert_ne!(call_thread, thread::current().id());
    assert_eq!(host.log.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn an_async_function_is_awaited_on_the_event_loop() {
    let host = FakeHost::new(LoadReply::Module(pending(allow_result())));
    apply(&host, ASYNC_SOURCE).await.unwrap();
    assert_eq!(host.log.calls.load(Ordering::SeqCst), 1);
    let call_thread = host.log.threads.lock().unwrap().first().copied().unwrap();
    assert_eq!(call_thread, thread::current().id());
}

#[tokio::test]
async fn a_sync_function_that_returns_an_awaitable_is_awaited() {
    let host = FakeHost::new(LoadReply::Module(pending(allow_result())));
    apply(&host, ALLOW_SOURCE).await.unwrap();
    assert_eq!(host.log.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_sync_function_past_the_deadline_fails_with_the_timeout() {
    let host = FakeHost::new(LoadReply::Module(spinning()));
    let guardrail = build(params_with_timeout(ALLOW_SOURCE, 0.03), &host);
    let started = Instant::now();
    let mut request_data = BTreeMap::new();
    let error = guardrail
        .apply_guardrail(
            inputs(&["hello"]),
            &mut request_data,
            GuardrailInputType::Request,
        )
        .await
        .unwrap_err();
    assert!(matches!(error, CustomCodeApplyError::Execution(_)));
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[tokio::test]
async fn an_async_function_past_the_deadline_fails_with_the_timeout() {
    let host = FakeHost::new(LoadReply::Module(never()));
    let guardrail = build(params_with_timeout(ASYNC_SOURCE, 0.03), &host);
    let started = Instant::now();
    let mut request_data = BTreeMap::new();
    let error = guardrail
        .apply_guardrail(
            inputs(&["hello"]),
            &mut request_data,
            GuardrailInputType::Request,
        )
        .await
        .unwrap_err();
    assert!(matches!(error, CustomCodeApplyError::Execution(_)));
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[tokio::test]
async fn an_awaitable_returned_by_a_sync_function_shares_the_deadline() {
    let host = FakeHost::new(LoadReply::Module(parking_then_pending(
        Duration::from_millis(300),
    )));
    let guardrail = build_with_clock(params_with_timeout(ALLOW_SOURCE, 0.5), &host, wall_clock);
    let started = Instant::now();
    let mut request_data = BTreeMap::new();
    let error = guardrail
        .apply_guardrail(
            inputs(&["hello"]),
            &mut request_data,
            GuardrailInputType::Request,
        )
        .await
        .unwrap_err();
    assert!(matches!(error, CustomCodeApplyError::Execution(_)));
    assert!(
        started.elapsed() < Duration::from_millis(700),
        "the awaitable got a fresh budget instead of what was left: {:?}",
        started.elapsed()
    );
}

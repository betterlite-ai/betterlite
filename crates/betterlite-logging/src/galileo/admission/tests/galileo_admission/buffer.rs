use tracing::Level;
use tracing::subscriber::with_default;
use tracing_subscriber::layer::SubscriberExt as _;

use super::{
    API_KEY_ENV, CapturedLogs, CountingIds, ScriptedTransport, chat_response, delivery_with_batch,
    event, kwargs, prefill, records, standard_object, text, user_messages,
};
use crate::galileo::admission::async_log_success_event;

#[test]
fn append_keeps_records_in_order() {
    let delivery = delivery_with_batch(API_KEY_ENV, 10);
    let response = chat_response("Hi");
    let transport = ScriptedTransport::default();
    let first = kwargs(Some(standard_object()), Some(user_messages("a")));
    let second = kwargs(Some(standard_object()), Some(user_messages("b")));

    async_log_success_event(
        &delivery,
        &transport,
        &CountingIds::default(),
        &event(&first, Some(&response)),
    );
    async_log_success_event(
        &delivery,
        &transport,
        &CountingIds::default(),
        &event(&second, Some(&response)),
    );

    let queued = records(&delivery);
    assert_eq!(queued.len(), 2);
    assert_eq!(queued[0].get("input_text"), Some(&text("a")));
    assert_eq!(queued[1].get("input_text"), Some(&text("b")));
}

#[test]
fn cap_drops_the_oldest_records() {
    let delivery = delivery_with_batch(API_KEY_ENV, 2000);
    prefill(&delivery, 1000);
    let response = chat_response("Hi");
    let transport = ScriptedTransport::default();
    let call_kwargs = kwargs(Some(standard_object()), Some(user_messages("new")));

    async_log_success_event(
        &delivery,
        &transport,
        &CountingIds::default(),
        &event(&call_kwargs, Some(&response)),
    );

    let queued = records(&delivery);
    assert_eq!(queued.len(), 1000);
    assert_eq!(queued[0].get("input_text"), Some(&text("1")));
    assert_eq!(queued[999].get("input_text"), Some(&text("new")));
}

#[test]
fn overflow_logs_a_warning() {
    let delivery = delivery_with_batch(API_KEY_ENV, 2000);
    prefill(&delivery, 1003);
    let response = chat_response("Hi");
    let transport = ScriptedTransport::default();
    let call_kwargs = kwargs(Some(standard_object()), Some(user_messages("new")));
    let logs = CapturedLogs::default();
    let subscriber = tracing_subscriber::registry().with(logs.clone());

    with_default(subscriber, || {
        async_log_success_event(
            &delivery,
            &transport,
            &CountingIds::default(),
            &event(&call_kwargs, Some(&response)),
        );
    });

    let warnings = logs.matching(Level::WARN, "in-memory buffer exceeded 1000 records");
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("dropped 4 oldest record(s)"));
}

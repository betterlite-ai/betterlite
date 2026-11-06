use super::{
    API_KEY_ENV, CountingIds, ScriptedTransport, chat_response, delivery, delivery_with_batch,
    event, kwargs, records, standard_object, user_messages,
};
use crate::galileo::admission::async_log_success_event;

#[test]
fn batch_threshold_triggers_a_flush() {
    let delivery = delivery_with_batch(API_KEY_ENV, 2);
    let transport = ScriptedTransport::replying(201);
    let response = chat_response("Hi");
    let call_kwargs = kwargs(Some(standard_object()), Some(user_messages("hi")));
    let call = event(&call_kwargs, Some(&response));

    async_log_success_event(&delivery, &transport, &CountingIds::default(), &call);
    assert_eq!(records(&delivery).len(), 1);
    assert!(transport.calls().is_empty());

    async_log_success_event(&delivery, &transport, &CountingIds::default(), &call);
    assert!(records(&delivery).is_empty());
    assert_eq!(transport.calls().len(), 1);
}

#[test]
fn the_default_batch_size_flushes_every_record() {
    let delivery = delivery(API_KEY_ENV);
    let transport = ScriptedTransport::replying(201);
    let response = chat_response("Hi");
    let call_kwargs = kwargs(Some(standard_object()), Some(user_messages("hi")));

    async_log_success_event(
        &delivery,
        &transport,
        &CountingIds::default(),
        &event(&call_kwargs, Some(&response)),
    );

    assert!(records(&delivery).is_empty());
    assert_eq!(transport.calls().len(), 1);
}

#[test]
fn below_the_threshold_does_not_flush() {
    let delivery = delivery_with_batch(API_KEY_ENV, 3);
    let transport = ScriptedTransport::replying(201);
    let response = chat_response("Hi");
    let call_kwargs = kwargs(Some(standard_object()), Some(user_messages("hi")));
    let call = event(&call_kwargs, Some(&response));

    async_log_success_event(&delivery, &transport, &CountingIds::default(), &call);
    async_log_success_event(&delivery, &transport, &CountingIds::default(), &call);

    assert_eq!(records(&delivery).len(), 2);
    assert!(transport.calls().is_empty());
}

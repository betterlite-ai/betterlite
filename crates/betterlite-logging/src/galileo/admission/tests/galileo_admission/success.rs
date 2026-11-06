use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use tracing::Level;
use tracing::subscriber::with_default;
use tracing_subscriber::layer::SubscriberExt as _;

use super::{
    API_KEY_ENV, CHAT_OUTPUT, CountingIds, ScriptedTransport, aware, chat_response, decimal,
    delivery, delivery_with_batch, event, kwargs, naive, number, offset_minutes, records,
    standard_object, text, user_messages,
};
use crate::galileo::admission::{CallbackKwargs, async_log_success_event};
use crate::galileo_payloads::content::ResponseObj;

#[test]
fn callback_kwargs_decode_from_the_wire_shape() {
    let wire = serde_json::json!({
        "standard_logging_object": {
            "call_type": "acompletion",
            "model": "gpt-5.2",
            "prompt_tokens": 3,
            "completion_tokens": 4,
            "total_tokens": 7,
            "response_cost": 0.001,
            "startTime": 1779710400.0,
            "endTime": 1779710401.5
        },
        "call_type": "acompletion",
        "model": "gpt-4o",
        "messages": [{"role": "user", "content": "hi"}]
    });

    let kwargs: CallbackKwargs = serde_json::from_value(wire).expect("the callback kwargs decode");
    let slo = kwargs
        .standard_logging_object
        .expect("the standard logging object decodes");

    assert_eq!(slo.prompt_tokens, Some(3));
    assert_eq!(slo.total_tokens, Some(7));
    assert_eq!(slo.response_cost, Some(Decimal::new(1, 3)));
    assert_eq!(slo.start_time, Some(Decimal::from(1_779_710_400_i64)));
    assert_eq!(slo.end_time, Some(Decimal::new(1_779_710_401_500, 3)));
    assert_eq!(kwargs.model.as_deref(), Some("gpt-4o"));
    assert_eq!(kwargs.content.call_type.as_deref(), Some("acompletion"));
    assert_eq!(kwargs.content.messages, Some(user_messages("hi")));
}

#[test]
fn success_callback_queues_a_record() {
    let delivery = delivery_with_batch(API_KEY_ENV, 10);
    let call_kwargs = kwargs(Some(standard_object()), Some(user_messages("hi")));
    let response = chat_response("Hi");
    let transport = ScriptedTransport::default();

    async_log_success_event(
        &delivery,
        &transport,
        &CountingIds::default(),
        &event(&call_kwargs, Some(&response)),
    );

    let queued = records(&delivery);
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].get("input_text"), Some(&text("hi")));
    assert_eq!(queued[0].get("output_text"), Some(&text(CHAT_OUTPUT)));
    assert_eq!(queued[0].get("node_type"), Some(&text("acompletion")));
    assert_eq!(queued[0].get("num_total_tokens"), Some(&number(7)));
    assert_eq!(
        queued[0].get("created_at"),
        Some(&text("2026-05-25T12:00:00Z"))
    );
}

#[test]
fn not_configured_skips_silently() {
    let delivery = delivery(&[]);
    let call_kwargs = kwargs(Some(standard_object()), Some(user_messages("hi")));
    let response = chat_response("Hi");
    let transport = ScriptedTransport::default();

    async_log_success_event(
        &delivery,
        &transport,
        &CountingIds::default(),
        &event(&call_kwargs, Some(&response)),
    );

    assert!(records(&delivery).is_empty());
    assert!(transport.calls().is_empty());
}

#[test]
fn missing_standard_logging_object_skips() {
    let delivery = delivery_with_batch(API_KEY_ENV, 10);
    let call_kwargs = kwargs(None, Some(user_messages("hi")));
    let response = chat_response("Hi");
    let transport = ScriptedTransport::default();

    async_log_success_event(
        &delivery,
        &transport,
        &CountingIds::default(),
        &event(&call_kwargs, Some(&response)),
    );

    assert!(records(&delivery).is_empty());
    assert!(transport.calls().is_empty());
}

#[test]
fn time_comes_from_standard_logging_object() {
    let delivery = delivery_with_batch(API_KEY_ENV, 10);
    let call_kwargs = kwargs(Some(standard_object()), Some(user_messages("hi")));
    let response = chat_response("Hi");
    let transport = ScriptedTransport::default();
    let mut call = event(&call_kwargs, Some(&response));
    call.local_offset = offset_minutes(120);

    async_log_success_event(&delivery, &transport, &CountingIds::default(), &call);

    let queued = records(&delivery);
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].get("latency_ms"), Some(&number(1500)));
    assert_eq!(
        queued[0].get("created_at"),
        Some(&text("2026-05-25T12:00:00Z"))
    );
}

#[test]
fn time_falls_back_to_callback_datetimes() {
    let mut slo = standard_object();
    slo.start_time = None;
    slo.end_time = None;
    let delivery = delivery_with_batch(API_KEY_ENV, 10);
    let call_kwargs = kwargs(Some(slo), Some(user_messages("hi")));
    let response = chat_response("Hi");
    let transport = ScriptedTransport::default();
    let mut local_call = event(&call_kwargs, Some(&response));
    local_call.start_time = Some(naive("2026-05-25T12:00:00"));
    local_call.end_time = Some(naive("2026-05-25T12:00:01.5"));
    local_call.local_offset = offset_minutes(120);
    let mut aware_call = event(&call_kwargs, Some(&response));
    aware_call.start_time = Some(aware("2026-05-25T12:00:00+02:00"));
    aware_call.end_time = Some(aware("2026-05-25T12:00:01.5+02:00"));

    async_log_success_event(&delivery, &transport, &CountingIds::default(), &local_call);
    async_log_success_event(&delivery, &transport, &CountingIds::default(), &aware_call);

    let queued = records(&delivery);
    assert_eq!(queued.len(), 2);
    assert_eq!(
        queued[0].get("created_at"),
        Some(&text("2026-05-25T10:00:00Z"))
    );
    assert_eq!(queued[0].get("latency_ms"), Some(&number(1500)));
    assert_eq!(
        queued[1].get("created_at"),
        Some(&text("2026-05-25T10:00:00Z"))
    );
}

#[test]
fn fallback_without_datetimes_is_dropped() {
    let mut slo = standard_object();
    slo.start_time = None;
    slo.end_time = None;
    let delivery = delivery_with_batch(API_KEY_ENV, 10);
    let call_kwargs = kwargs(Some(slo), Some(user_messages("hi")));
    let response = chat_response("Hi");
    let transport = ScriptedTransport::default();

    async_log_success_event(
        &delivery,
        &transport,
        &CountingIds::default(),
        &event(&call_kwargs, Some(&response)),
    );

    assert!(records(&delivery).is_empty());
}

#[test]
fn token_counts_fall_back_to_zero() {
    let mut slo = standard_object();
    slo.prompt_tokens = None;
    slo.completion_tokens = None;
    slo.total_tokens = Some(5);
    let delivery = delivery_with_batch(API_KEY_ENV, 10);
    let call_kwargs = kwargs(Some(slo), Some(user_messages("hi")));
    let response = chat_response("Hi");
    let transport = ScriptedTransport::default();

    async_log_success_event(
        &delivery,
        &transport,
        &CountingIds::default(),
        &event(&call_kwargs, Some(&response)),
    );

    let queued = records(&delivery);
    assert_eq!(queued[0].get("num_input_tokens"), Some(&number(0)));
    assert_eq!(queued[0].get("num_output_tokens"), Some(&number(0)));
    assert_eq!(queued[0].get("num_total_tokens"), Some(&number(5)));
}

#[test]
fn total_tokens_is_computed_from_components() {
    let mut zero_total = standard_object();
    zero_total.prompt_tokens = Some(1);
    zero_total.completion_tokens = Some(2);
    zero_total.total_tokens = Some(0);
    let mut missing_total = standard_object();
    missing_total.prompt_tokens = Some(1);
    missing_total.completion_tokens = Some(2);
    missing_total.total_tokens = None;
    let delivery = delivery_with_batch(API_KEY_ENV, 10);
    let response = chat_response("Hi");
    let transport = ScriptedTransport::default();

    let zero_kwargs = kwargs(Some(zero_total), Some(user_messages("hi")));
    async_log_success_event(
        &delivery,
        &transport,
        &CountingIds::default(),
        &event(&zero_kwargs, Some(&response)),
    );
    let missing_kwargs = kwargs(Some(missing_total), Some(user_messages("hi")));
    async_log_success_event(
        &delivery,
        &transport,
        &CountingIds::default(),
        &event(&missing_kwargs, Some(&response)),
    );

    let queued = records(&delivery);
    assert_eq!(queued.len(), 2);
    assert_eq!(queued[0].get("num_total_tokens"), Some(&number(3)));
    assert_eq!(queued[1].get("num_total_tokens"), Some(&number(3)));
}

#[test]
fn record_carries_every_required_field() {
    let delivery = delivery_with_batch(API_KEY_ENV, 10);
    let call_kwargs = kwargs(Some(standard_object()), Some(user_messages("hi")));
    let response = chat_response("Hi");
    let transport = ScriptedTransport::default();

    async_log_success_event(
        &delivery,
        &transport,
        &CountingIds::default(),
        &event(&call_kwargs, Some(&response)),
    );

    let queued = records(&delivery);
    let record = &queued[0];
    assert_eq!(record.get("latency_ms"), Some(&number(1500)));
    assert_eq!(record.get("status_code"), Some(&number(200)));
    assert_eq!(record.get("input_text"), Some(&text("hi")));
    assert_eq!(record.get("output_text"), Some(&text(CHAT_OUTPUT)));
    assert_eq!(record.get("node_type"), Some(&text("acompletion")));
    assert_eq!(record.get("model"), Some(&text("gpt-5.2")));
    assert_eq!(record.get("num_input_tokens"), Some(&number(3)));
    assert_eq!(record.get("num_output_tokens"), Some(&number(4)));
    assert_eq!(record.get("num_total_tokens"), Some(&number(7)));
    assert_eq!(record.get("cost"), Some(&decimal("0.001")));
    assert_eq!(
        record.get("created_at"),
        Some(&text("2026-05-25T12:00:00Z"))
    );
}

#[test]
fn record_leaves_unset_optional_fields_out() {
    let delivery = delivery_with_batch(API_KEY_ENV, 10);
    let call_kwargs = kwargs(Some(standard_object()), Some(user_messages("hi")));
    let response = chat_response("Hi");
    let transport = ScriptedTransport::default();

    async_log_success_event(
        &delivery,
        &transport,
        &CountingIds::default(),
        &event(&call_kwargs, Some(&response)),
    );

    let queued = records(&delivery);
    let record = &queued[0];
    assert!(!record.contains_key("output_logprobs"));
    assert!(!record.contains_key("tags"));
    assert!(!record.contains_key("user_metadata"));
    assert!(!record.contains_key("messages"));
}

#[test]
fn call_type_and_model_fall_back_to_kwargs_then_defaults() {
    let mut from_kwargs_slo = standard_object();
    from_kwargs_slo.call_type = None;
    from_kwargs_slo.model = None;
    let mut bare_slo = standard_object();
    bare_slo.call_type = None;
    bare_slo.model = None;
    let delivery = delivery_with_batch(API_KEY_ENV, 10);
    let response = chat_response("Hi");
    let transport = ScriptedTransport::default();
    let from_kwargs = kwargs(Some(from_kwargs_slo), None);
    let mut bare = kwargs(Some(bare_slo), None);
    bare.model = None;
    bare.content.call_type = None;

    async_log_success_event(
        &delivery,
        &transport,
        &CountingIds::default(),
        &event(&from_kwargs, Some(&response)),
    );
    async_log_success_event(
        &delivery,
        &transport,
        &CountingIds::default(),
        &event(&bare, Some(&response)),
    );

    let queued = records(&delivery);
    assert_eq!(queued[0].get("node_type"), Some(&text("acompletion")));
    assert_eq!(queued[0].get("model"), Some(&text("gpt-4o")));
    assert_eq!(queued[1].get("node_type"), Some(&text("litellm")));
    assert_eq!(queued[1].get("model"), Some(&text("-")));
}

#[test]
fn messages_reach_the_record_only_when_present() {
    let delivery = delivery_with_batch(API_KEY_ENV, 10);
    let response = chat_response("Hi");
    let transport = ScriptedTransport::default();
    let with_messages = kwargs(Some(standard_object()), Some(user_messages("hi")));
    let without_messages = kwargs(Some(standard_object()), None);
    let mut pass_through = kwargs(Some(standard_object()), Some(user_messages("hi")));
    pass_through.content.call_type = Some("pass_through_endpoint".to_owned());
    let mut value = IndexMap::new();
    value.insert("response".to_owned(), Json::Text("ok".to_owned()));
    let pass_through_response = ResponseObj::PassThroughDict { value };

    async_log_success_event(
        &delivery,
        &transport,
        &CountingIds::default(),
        &event(&with_messages, Some(&response)),
    );
    async_log_success_event(
        &delivery,
        &transport,
        &CountingIds::default(),
        &event(&without_messages, Some(&response)),
    );
    async_log_success_event(
        &delivery,
        &transport,
        &CountingIds::default(),
        &event(&pass_through, Some(&pass_through_response)),
    );

    let queued = records(&delivery);
    assert_eq!(queued.len(), 3);
    assert_eq!(queued[0].get("messages"), Some(&user_messages("hi")));
    assert!(!queued[1].contains_key("messages"));
    assert_eq!(queued[2].get("messages"), Some(&user_messages("hi")));
}

#[test]
fn runtime_errors_are_logged_and_swallowed() {
    let mut slo = standard_object();
    slo.start_time = Some(Decimal::from(1_000_000_000_000_000_i64));
    let delivery = delivery_with_batch(API_KEY_ENV, 10);
    let call_kwargs = kwargs(Some(slo), Some(user_messages("hi")));
    let response = chat_response("Hi");
    let transport = ScriptedTransport::default();
    let logs = super::CapturedLogs::default();
    let subscriber = tracing_subscriber::registry().with(logs.clone());

    with_default(subscriber, || {
        async_log_success_event(
            &delivery,
            &transport,
            &CountingIds::default(),
            &event(&call_kwargs, Some(&response)),
        );
    });

    assert!(records(&delivery).is_empty());
    assert!(transport.calls().is_empty());
    assert_eq!(logs.matching(Level::ERROR, "unexpected error").len(), 1);
}

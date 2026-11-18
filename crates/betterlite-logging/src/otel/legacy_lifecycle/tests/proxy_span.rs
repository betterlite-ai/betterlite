use betterlite_logging::otel::legacy_lifecycle::lifecycle::LifecycleEvent;
use opentelemetry::Context;
use opentelemetry::trace::{Span as _, Status};

use crate::support::{
    END, PROXY, Rig, attach, attribute, call, carry_in, instant, simple_kwargs, text_attribute,
    with_team,
};

#[test]
fn proxy_span_success_close_stamps_status_200_and_ok() {
    let rig = Rig::new();
    let lifecycle = rig.in_memory(None);
    let proxy = rig.parent(&lifecycle, PROXY);
    let mut kwargs = simple_kwargs("call-1");
    carry_in(&mut kwargs, "metadata", &proxy);
    lifecycle
        .handle(&call(&kwargs), LifecycleEvent::Success)
        .expect("success lifecycle");
    let span = rig.span(PROXY);
    assert_eq!(span.status, Status::Ok);
    assert_eq!(span.end_time, instant(END));
    assert_eq!(
        attribute(&span, "http.response.status_code"),
        Some(&opentelemetry::Value::I64(200))
    );
    assert_eq!(
        rig.span("litellm_request").parent_span_id,
        proxy.context.span_id(),
        "the primary span hangs off the proxy span"
    );
}

#[test]
fn proxy_span_success_leaves_external_parent_spans_open() {
    let rig = Rig::new();
    let lifecycle = rig.in_memory(None);
    let parent = rig.parent(&lifecycle, "upstream");
    let mut kwargs = simple_kwargs("call-2");
    carry_in(&mut kwargs, "metadata", &parent);
    lifecycle
        .handle(&call(&kwargs), LifecycleEvent::Success)
        .expect("success lifecycle");
    assert!(
        !rig.names().iter().any(|name| name == "upstream"),
        "an externally provided span is never closed by the lifecycle"
    );
    assert!(parent.lock().is_recording());
}

#[test]
fn proxy_span_success_does_not_close_an_ended_proxy_again() {
    let rig = Rig::new();
    let lifecycle = rig.in_memory(None);
    let proxy = rig.parent(&lifecycle, PROXY);
    proxy.lock().end_with_timestamp(instant(END - 0.5));
    let mut kwargs = simple_kwargs("call-ended-proxy");
    carry_in(&mut kwargs, "metadata", &proxy);

    lifecycle
        .handle(&call(&kwargs), LifecycleEvent::Success)
        .expect("success lifecycle");

    let span = rig.span(PROXY);
    assert_eq!(span.status, Status::Unset);
    assert_eq!(span.end_time, instant(END - 0.5));
    assert_eq!(attribute(&span, "http.response.status_code"), None);
}

#[test]
fn proxy_span_team_attributes_are_stamped_before_the_close() {
    let rig = Rig::new();
    let lifecycle = rig.in_memory(None);
    let proxy = rig.parent(&lifecycle, PROXY);
    let mut kwargs = simple_kwargs("call-3");
    carry_in(&mut kwargs, "metadata", &proxy);
    with_team(&mut kwargs);
    lifecycle
        .handle(&call(&kwargs), LifecycleEvent::Success)
        .expect("success lifecycle");
    let span = rig.span(PROXY);
    assert_eq!(
        text_attribute(&span, "metadata.user_api_key_team_id"),
        "team-a"
    );
    assert_eq!(
        text_attribute(&span, "metadata.user_api_key_team_alias"),
        "team-alias"
    );
    assert_eq!(span.status, Status::Ok);
}

#[test]
fn proxy_span_failure_closes_the_ambient_proxy_without_a_status_stamp() {
    let rig = Rig::new();
    let lifecycle = rig.in_memory(None);
    let proxy = rig.parent(&lifecycle, PROXY);
    let kwargs = simple_kwargs("call-4");
    let context = attach(&proxy.context);
    let guard = Context::attach(&context);
    lifecycle
        .handle(&call(&kwargs), LifecycleEvent::Failure)
        .expect("failure lifecycle");
    drop(guard);
    let span = rig.span(PROXY);
    assert!(matches!(span.status, Status::Error { .. }));
    assert_eq!(span.end_time, instant(END));
    assert_eq!(attribute(&span, "http.response.status_code"), None);
}

#[test]
fn proxy_span_success_dedupe_skip_still_closes_the_proxy() {
    let rig = Rig::new();
    let lifecycle = rig.in_memory(None);
    let proxy = rig.parent(&lifecycle, PROXY);
    let first = simple_kwargs("call-5");
    lifecycle
        .handle(&call(&first), LifecycleEvent::Success)
        .expect("first success");
    assert!(proxy.lock().is_recording(), "no carrier closes nothing");
    let mut second = first.clone();
    carry_in(&mut second, "metadata", &proxy);
    lifecycle
        .handle(&call(&second), LifecycleEvent::Success)
        .expect("duplicate success");
    let span = rig.span(PROXY);
    assert_eq!(span.status, Status::Ok);
    assert_eq!(
        attribute(&span, "http.response.status_code"),
        Some(&opentelemetry::Value::I64(200))
    );
}

#[test]
fn proxy_span_success_closes_the_ambient_proxy_span() {
    let rig = Rig::new();
    let lifecycle = rig.in_memory(None);
    let proxy = rig.parent(&lifecycle, PROXY);
    let kwargs = simple_kwargs("call-ambient-proxy");
    let context = attach(&proxy.context);
    let guard = Context::attach(&context);
    lifecycle
        .handle(&call(&kwargs), LifecycleEvent::Success)
        .expect("success lifecycle");
    drop(guard);
    let span = rig.span(PROXY);
    assert_eq!(span.status, Status::Ok);
    assert_eq!(span.end_time, instant(END));
    assert_eq!(
        attribute(&span, "http.response.status_code"),
        Some(&opentelemetry::Value::I64(200))
    );
}

#[test]
fn proxy_span_team_attributes_read_only_the_provider_bucket() {
    let rig = Rig::new();
    let lifecycle = rig.in_memory(None);
    let proxy = rig.parent(&lifecycle, PROXY);
    let mut kwargs = simple_kwargs("call-litellm-metadata");
    carry_in(&mut kwargs, "litellm_metadata", &proxy);
    with_team(&mut kwargs);
    lifecycle
        .handle(&call(&kwargs), LifecycleEvent::Success)
        .expect("success lifecycle");
    let span = rig.span(PROXY);
    assert_eq!(span.status, Status::Ok);
    assert_eq!(
        attribute(&span, "metadata.user_api_key_team_id"),
        None,
        "the litellm_metadata carrier is only the close fallback"
    );
}

#[test]
fn proxy_span_null_carrier_uses_the_proxy_bucket() {
    use betterlite_logging::otel::model::utils::coercion::OtelModelValue;

    let rig = Rig::new();
    let lifecycle = rig.in_memory(None);
    let proxy = rig.parent(&lifecycle, PROXY);
    let mut kwargs = simple_kwargs("null-carrier");
    carry_in(&mut kwargs, "litellm_metadata", &proxy);
    let Some(OtelModelValue::Object(params)) = kwargs.get_mut("litellm_params") else {
        panic!("litellm_params must be an object");
    };
    let Some(OtelModelValue::Object(metadata)) = params.get_mut("metadata") else {
        panic!("metadata must be an object");
    };
    metadata.insert(
        "litellm_parent_otel_span".to_owned(),
        OtelModelValue::Null(()),
    );

    lifecycle
        .handle(&call(&kwargs), LifecycleEvent::Success)
        .expect("success");

    assert!(!proxy.lock().is_recording());
    let span = rig.span(PROXY);
    assert_eq!(span.status, Status::Ok);
    assert_eq!(
        attribute(&span, "http.response.status_code"),
        Some(&opentelemetry::Value::I64(200))
    );
}

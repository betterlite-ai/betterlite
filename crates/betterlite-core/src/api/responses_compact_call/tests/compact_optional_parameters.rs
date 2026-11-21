use super::support::{
    COMPACT_OK, Harness, Rig, base_env, body_json, call, json, kwargs, object, text_input,
};
use crate::api::responses_compact_call::compact_responses;
use crate::api::responses_compact_call::error::CompactResponsesError;
use crate::api::responses_compact_http::service::LogContext;
use crate::api::responses_parameters::error::UNSUPPORTED_PARAMS_STATUS;

pub(super) async fn forwarded() {
    let mut harness = Harness::new();
    let mut rig = Rig::default();
    rig.wire.queue(200, COMPACT_OK);
    let prepost = base_env(&harness.redactor, &harness.sink, harness.now);
    let mut log = LogContext {
        state: &mut harness.state,
        env: &prepost,
    };
    let mut full = call("openai/gpt-4o", text_input("hello"));
    full.instructions = Some(String::from("short"));
    full.kwargs = kwargs(r#"{"temperature":0.2,"trace":"ignored"}"#);
    full.extra_query = Some(object(r#"{"trace":"ignored"}"#));
    full.extra_body = Some(object(r#"{"model":"ignored"}"#));
    compact_responses(&rig.deps(), &full, &mut log)
        .await
        .expect("the supported options reach the provider");

    let requests = rig.wire.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        body_json(&requests[0]),
        json(r#"{"model":"gpt-4o","input":"hello","temperature":0.2,"instructions":"short"}"#)
    );
    assert_eq!(requests[0].url.query(), None);
}

pub(super) async fn unsupported_rejected() {
    let mut harness = Harness::new();
    let mut rig = Rig::default();
    rig.configs.supported = Some(vec![String::from("instructions")]);
    let prepost = base_env(&harness.redactor, &harness.sink, harness.now);
    let mut log = LogContext {
        state: &mut harness.state,
        env: &prepost,
    };
    let mut rejected = call("openai/gpt-4o", text_input("hello"));
    rejected.kwargs = kwargs(r#"{"temperature":0.2}"#);
    let Err(error) = compact_responses(&rig.deps(), &rejected, &mut log).await else {
        panic!("an unsupported parameter must be rejected before the request");
    };
    let failure = error.sdk_failure("openai", "gpt-4o", &harness.redactor);
    assert_eq!(failure.class, "UnsupportedParamsError");
    assert_eq!(failure.status, UNSUPPORTED_PARAMS_STATUS);
    assert!(failure.kind.is_none());
    assert!(failure.param.is_none());
    assert!(failure.code.is_none());
    assert!(rig.wire.requests().is_empty());
}

pub(super) async fn drop_params_normalized() {
    let mut harness = Harness::new();
    let mut rig = Rig::default();
    rig.configs.supported = Some(vec![String::from("instructions")]);
    rig.wire.queue(200, COMPACT_OK);
    let prepost = base_env(&harness.redactor, &harness.sink, harness.now);
    let mut log = LogContext {
        state: &mut harness.state,
        env: &prepost,
    };
    let mut dropping = call("openai/gpt-4o", text_input("hello"));
    dropping.instructions = Some(String::from("short"));
    dropping.kwargs = kwargs(r#"{"temperature":0.2,"drop_params":true}"#);
    compact_responses(&rig.deps(), &dropping, &mut log)
        .await
        .expect("drop_params true drops the unsupported parameter");

    let mut text_dropping = call("openai/gpt-4o", text_input("hello"));
    text_dropping.kwargs = kwargs(r#"{"temperature":0.2,"drop_params":"false"}"#);
    let Err(text_error) = compact_responses(&rig.deps(), &text_dropping, &mut log).await else {
        panic!("the text spelling false must not drop");
    };
    assert!(matches!(text_error, CompactResponsesError::Param(_)));
    let requests = rig.wire.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        body_json(&requests[0]),
        json(r#"{"model":"gpt-4o","input":"hello","instructions":"short"}"#)
    );
}

pub(super) async fn global_drop_params() {
    let mut harness = Harness::new();
    let mut rig = Rig::default();
    rig.configs.supported = Some(vec![String::from("instructions")]);
    rig.drop_params = true;
    rig.wire.queue(200, COMPACT_OK);
    let prepost = base_env(&harness.redactor, &harness.sink, harness.now);
    let mut log = LogContext {
        state: &mut harness.state,
        env: &prepost,
    };
    let mut dropping = call("openai/gpt-4o", text_input("hello"));
    dropping.instructions = Some(String::from("short"));
    dropping.kwargs = kwargs(r#"{"temperature":0.2}"#);
    compact_responses(&rig.deps(), &dropping, &mut log)
        .await
        .expect("the global drop_params drops the unsupported parameter");

    let requests = rig.wire.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        body_json(&requests[0]),
        json(r#"{"model":"gpt-4o","input":"hello","instructions":"short"}"#)
    );
}

pub(super) async fn bypass_and_nulls() {
    let mut harness = Harness::new();
    let mut rig = Rig::default();
    rig.configs.supported = Some(vec![String::from("instructions")]);
    rig.wire.repeat(200, COMPACT_OK, 2);
    let prepost = base_env(&harness.redactor, &harness.sink, harness.now);
    let mut log = LogContext {
        state: &mut harness.state,
        env: &prepost,
    };
    let mut bypass = call("openai/gpt-4o", text_input("hello"));
    bypass.kwargs = kwargs(r#"{"temperature":0.2,"allowed_openai_params":["temperature"]}"#);
    let Err(bypass_error) = compact_responses(&rig.deps(), &bypass, &mut log).await else {
        panic!("compact never reads the caller's allowed_openai_params");
    };
    assert!(matches!(bypass_error, CompactResponsesError::Param(_)));

    rig.configs.supported = None;
    let mut nulled = call("openai/gpt-4o", text_input("hello"));
    nulled.kwargs = kwargs(r#"{"temperature":null}"#);
    compact_responses(&rig.deps(), &nulled, &mut log)
        .await
        .expect("a null optional value is omitted");
    let mut unknown = call("openai/gpt-4o", text_input("hello"));
    unknown.kwargs = kwargs(r#"{"top_k":5,"trace":"t"}"#);
    compact_responses(&rig.deps(), &unknown, &mut log)
        .await
        .expect("unknown kwargs are filtered");

    let requests = rig.wire.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        body_json(&requests[0]),
        json(r#"{"model":"gpt-4o","input":"hello"}"#)
    );
    assert_eq!(
        body_json(&requests[1]),
        json(r#"{"model":"gpt-4o","input":"hello"}"#)
    );
}

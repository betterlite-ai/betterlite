use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;

use super::support::{
    COMPACT_OK, Events, Harness, ModelNames, Rig, base_env, call, env_vars, kwargs, text_input,
};
use crate::api::responses_compact_call::error::CompactResponsesError;
use crate::api::responses_compact_call::{acompact_responses, compact_responses};
use crate::api::responses_compact_http::service::{LogContext, SharedSession};

pub(super) async fn default_timeouts() {
    let mut harness = Harness::new();
    let rig = Rig::default();
    rig.wire.repeat(200, COMPACT_OK, 3);
    let prepost = base_env(&harness.redactor, &harness.sink, harness.now);
    let mut log = LogContext {
        state: &mut harness.state,
        env: &prepost,
    };
    let mut compacting = call("openai/gpt-4o", text_input("hello"));
    compact_responses(&rig.deps(), &compacting, &mut log)
        .await
        .expect("the default timeout call succeeds");
    compacting.timeout = Some(0.0_f64);
    compact_responses(&rig.deps(), &compacting, &mut log)
        .await
        .expect("the zero timeout call succeeds");
    compacting.timeout = Some(7.0_f64);
    compact_responses(&rig.deps(), &compacting, &mut log)
        .await
        .expect("the explicit timeout call succeeds");

    let requests = rig.wire.requests();
    assert_eq!(requests[0].timeout, Some(Duration::from_mins(100)));
    assert_eq!(requests[1].timeout, Some(Duration::from_mins(100)));
    assert_eq!(requests[2].timeout, Some(Duration::from_secs(7)));
}

pub(super) async fn configured_timeouts() {
    let mut harness = Harness::new();
    let mut rig = Rig::default();
    rig.wire.repeat(200, COMPACT_OK, 2);
    rig.variables = env_vars(&[("OPENAI_API_KEY", "sk-provider"), ("REQUEST_TIMEOUT", "17")]);
    let prepost = base_env(&harness.redactor, &harness.sink, harness.now);
    let mut log = LogContext {
        state: &mut harness.state,
        env: &prepost,
    };
    let mut compacting = call("openai/gpt-4o", text_input("hello"));
    compact_responses(&rig.deps(), &compacting, &mut log)
        .await
        .expect("the configured timeout call succeeds");
    compacting.timeout = Some(0.0_f64);
    compact_responses(&rig.deps(), &compacting, &mut log)
        .await
        .expect("the configured zero timeout call succeeds");

    let requests = rig.wire.requests();
    assert_eq!(requests[0].timeout, Some(Duration::from_secs(17)));
    assert_eq!(requests[1].timeout, Some(Duration::from_secs(17)));
    rig.variables = env_vars(&[
        ("OPENAI_API_KEY", "sk-provider"),
        ("REQUEST_TIMEOUT", "soon"),
    ]);
    let Err(timeout_error) = compact_responses(&rig.deps(), &compacting, &mut log).await else {
        panic!("an unreadable REQUEST_TIMEOUT fails the call");
    };
    assert!(matches!(timeout_error, CompactResponsesError::Timeout(_)));
}

pub(super) async fn residency() {
    let mut harness = Harness::new();
    let rig = Rig::default();
    rig.wire.repeat(200, COMPACT_OK, 3);
    let prepost = base_env(&harness.redactor, &harness.sink, harness.now);
    let mut log = LogContext {
        state: &mut harness.state,
        env: &prepost,
    };
    let mut regional = call("openai/gpt-4o", text_input("hello"));
    regional.api_base = Some(String::from("https://eu.api.openai.com/v1"));
    compact_responses(&rig.deps(), &regional, &mut log)
        .await
        .expect("the eu host call succeeds");
    assert_eq!(
        log.state.litellm_params.get("data_residency"),
        Some(&Json::Text(String::from("eu")))
    );
    regional.api_base = Some(String::from("https://us.api.openai.com/v1"));
    compact_responses(&rig.deps(), &regional, &mut log)
        .await
        .expect("the us host call succeeds");
    assert_eq!(
        log.state.litellm_params.get("data_residency"),
        Some(&Json::Text(String::from("us")))
    );
    regional.api_base = Some(String::from("https://api.openai.com/v1"));
    compact_responses(&rig.deps(), &regional, &mut log)
        .await
        .expect("the global host call succeeds");
    assert_eq!(
        log.state.litellm_params.get("data_residency"),
        Some(&Json::Null)
    );
}

pub(super) async fn async_handoff() {
    let mut harness = Harness::new();
    let rig = Rig::default();
    rig.wire.queue(200, COMPACT_OK);
    let prepost = base_env(&harness.redactor, &harness.sink, harness.now);
    let mut log = LogContext {
        state: &mut harness.state,
        env: &prepost,
    };
    let mut handoff = call("openai/gpt-4o", text_input("hello"));
    handoff.api_base = Some(String::from("https://eu.api.openai.com/v1"));
    handoff.instructions = Some(String::from("short"));
    handoff.kwargs = kwargs(r#"{"litellm_call_id":"call-a"}"#);
    assert!(!rig.events.recorded().contains(&String::from("off_loop")));
    acompact_responses(&rig.deps(), &handoff, &mut log)
        .await
        .expect("the async handoff call succeeds");
    assert!(rig.events.recorded().contains(&String::from("off_loop")));
    handoff_context(log.state);
    preliminary_lookup(&rig.resolver);
}

fn handoff_context(state: &LoggingState) {
    assert_eq!(
        state.litellm_params.get("litellm_call_id"),
        Some(&Json::Text(String::from("call-a")))
    );
    assert_eq!(
        state.optional_params.get("instructions"),
        Some(&Json::Text(String::from("short")))
    );
    assert_eq!(
        state.litellm_params.get("input"),
        Some(&Json::Text(String::from("hello")))
    );
}

fn preliminary_lookup(resolver: &ModelNames) {
    let seen = resolver.seen();
    let matching = seen
        .iter()
        .filter(|resolution| resolution.model == "openai/gpt-4o")
        .collect::<Vec<_>>();
    assert!(
        matching[matching.len() - 2].api_base.is_none(),
        "the async wrapper's preliminary lookup reads no base"
    );
    assert_eq!(
        matching[matching.len() - 1].api_base,
        Some(String::from("https://eu.api.openai.com/v1"))
    );
}

pub(super) async fn explicit_provider_skips_lookup() {
    let mut harness = Harness::new();
    let rig = Rig::default();
    rig.wire.queue(200, COMPACT_OK);
    let prepost = base_env(&harness.redactor, &harness.sink, harness.now);
    let mut log = LogContext {
        state: &mut harness.state,
        env: &prepost,
    };
    let mut explicit = call("openai/gpt-4o", text_input("hello"));
    explicit.custom_llm_provider = Some(String::from("openai"));
    acompact_responses(&rig.deps(), &explicit, &mut log)
        .await
        .expect("the async call with an explicit provider succeeds");

    let seen = rig.resolver.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].custom_llm_provider.as_deref(), Some("openai"));
}

pub(super) async fn client_parameters() {
    let mut harness = Harness::new();
    let rig = Rig::default();
    rig.wire.queue(200, COMPACT_OK);
    let prepost = base_env(&harness.redactor, &harness.sink, harness.now);
    let mut log = LogContext {
        state: &mut harness.state,
        env: &prepost,
    };
    let mut secured = call("openai/gpt-4o", text_input("hello"));
    secured.extra_headers = Some(BTreeMap::from([(
        String::from("X-Trace"),
        String::from("trace-a"),
    )]));
    secured.ssl_verify = Some(false);
    secured.shared_session = Some(SharedSession(9));
    secured.acompact_responses = Some(Json::Bool(true));
    acompact_responses(&rig.deps(), &secured, &mut log)
        .await
        .expect("the async call with a shared session succeeds");

    let requests = rig.wire.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0]
            .headers
            .get("x-trace")
            .and_then(|value| value.to_str().ok()),
        Some("trace-a")
    );
    assert_eq!(rig.source.created(), vec![(Some(false), true)]);
}

pub(super) async fn sync_markers() {
    let mut harness = Harness::new();
    let rig = Rig::default();
    rig.wire.repeat(200, COMPACT_OK, 2);
    let prepost = base_env(&harness.redactor, &harness.sink, harness.now);
    let mut log = LogContext {
        state: &mut harness.state,
        env: &prepost,
    };
    let mut marked_off = call("openai/gpt-4o", text_input("hello"));
    marked_off.acompact_responses = Some(Json::Bool(false));
    compact_responses(&rig.deps(), &marked_off, &mut log)
        .await
        .expect("the false marker call succeeds");
    let mut marked_text = call("openai/gpt-4o", text_input("hello"));
    marked_text.acompact_responses = Some(Json::Text(String::from("true")));
    compact_responses(&rig.deps(), &marked_text, &mut log)
        .await
        .expect("the text marker call succeeds");

    assert_eq!(rig.wire.requests().len(), 2);
    assert_eq!(rig.source.created().len(), 2);
    assert_eq!(off_loop_count(&rig.events), 0);
}

pub(super) async fn true_marker() {
    let mut harness = Harness::new();
    let rig = Rig::default();
    rig.wire.queue(200, COMPACT_OK);
    let prepost = base_env(&harness.redactor, &harness.sink, harness.now);
    let mut log = LogContext {
        state: &mut harness.state,
        env: &prepost,
    };
    let mut marked_on = call("openai/gpt-4o", text_input("hello"));
    marked_on.acompact_responses = Some(Json::Bool(true));
    compact_responses(&rig.deps(), &marked_on, &mut log)
        .await
        .expect("the true marker call succeeds");

    assert_eq!(rig.wire.requests().len(), 1);
    assert_eq!(rig.source.created().len(), 1);
    assert_eq!(
        off_loop_count(&rig.events),
        1,
        "the boolean true marker selects the async path"
    );
}

fn off_loop_count(events: &Events) -> usize {
    events
        .recorded()
        .iter()
        .filter(|event| event.as_str() == "off_loop")
        .count()
}

#![cfg(test)]

mod support;

use std::time::Duration;

use betterlite_models::decisions::Json;

use crate::api::responses_compact_call::deps::CompactResponsesDeps;
use crate::api::responses_compact_call::error::CompactResponsesError;
use crate::api::responses_compact_call::{acompact_responses, compact_responses};
use crate::api::responses_compact_http::service::LogContext;
use crate::api::responses_parameters::error::UNSUPPORTED_PARAMS_STATUS;

use self::support::{
    COMPACT_OK, Configs, ENCODED_RESPONSE, Events, Harness, ModelNames, Registry, Source, Wire,
    base_env, body_json, call, env_vars, item_input, json, kwargs, object, text_input, typed_id,
};

const ITEM_INPUT: &str = r#"[{"type":"message","role":"user","id":"encitem_bGl0ZWxsbTptb2RlbF9pZDpkZXBsb3ltZW50X2E7aXRlbV9pZDptc2dfYWJj","content":[{"type":"input_text","text":"hello"}],"encrypted_content":"litellm_enc:bW9kZWxfaWQ6ZGVwbG95bWVudF9h;gAAAA-secret"}]"#;
const RESTORED_BODY: &str = r#"{"model":"gpt-4o","input":[{"type":"message","role":"user","id":"msg_abc","content":[{"type":"input_text","text":"hello"}],"encrypted_content":"gAAAA-secret"}],"instructions":"short","previous_response_id":"resp_previous"}"#;

#[tokio::test]
async fn compact_model_and_encrypted_items() {
    let events = Events::new();
    let mut harness = Harness::new();
    let wire = Wire::new(&events);
    wire.queue(200, COMPACT_OK);
    wire.queue(200, COMPACT_OK);
    wire.queue(200, COMPACT_OK);
    let source = Source::new(&wire);
    let resolver = ModelNames::default();
    let lookup = Registry;
    let configs = Configs {
        events: events.clone(),
        supported: None,
    };
    let variables = env_vars(&[("OPENAI_API_KEY", "sk-provider")]);
    let deps = CompactResponsesDeps {
        resolver: &resolver,
        lookup: &lookup,
        configs: &configs,
        source: &source,
        environment: &variables,
        drop_params: false,
    };
    let prepost = base_env(&harness.redactor, &harness.sink, harness.now);
    let mut log = LogContext {
        state: &mut harness.state,
        env: &prepost,
    };
    let mut compacting = call("openai/gpt-4o", item_input(ITEM_INPUT));
    compacting.instructions = Some(String::from("short"));
    compacting.previous_response_id = Some(String::from("resp_previous"));
    compacting.kwargs = kwargs(
        r#"{"litellm_metadata":{"model_info":{"id":"deployment_a"},"encrypted_content_affinity_enabled":true}}"#,
    );
    let sync_reply = compact_responses(&deps, &compacting, &mut log)
        .await
        .expect("the sync compact call succeeds");
    let async_reply = acompact_responses(&deps, &compacting, &mut log)
        .await
        .expect("the async compact call succeeds");
    let mut routed = call("responses/gpt-4o", text_input("hello"));
    routed.custom_llm_provider = Some(String::from("openai"));
    compact_responses(&deps, &routed, &mut log)
        .await
        .expect("the routed compact call succeeds");

    assert_eq!(typed_id(&sync_reply), ENCODED_RESPONSE);
    assert_eq!(typed_id(&async_reply), ENCODED_RESPONSE);
    let requests = wire.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(
        requests[0].url.as_str(),
        "http://mock.test/v1/responses/compact"
    );
    assert_eq!(body_json(&requests[0]), json(RESTORED_BODY));
    assert_eq!(body_json(&requests[1]), json(RESTORED_BODY));
    assert_eq!(
        body_json(&requests[2]),
        json(r#"{"model":"gpt-4o","input":"hello"}"#)
    );
    assert_eq!(
        requests[0]
            .headers
            .get("authorization")
            .and_then(|value| value.to_str().ok()),
        Some("Bearer sk-provider")
    );
    let seen = resolver.seen();
    let routed_seen = seen.last().expect("the routed call resolves once");
    assert_eq!(routed_seen.model, "responses/gpt-4o");
    assert_eq!(routed_seen.custom_llm_provider.as_deref(), Some("openai"));
}

#[tokio::test]
async fn compact_optional_parameters() {
    let events = Events::new();
    let mut harness = Harness::new();
    let wire = Wire::new(&events);
    wire.repeat(200, COMPACT_OK, 4);
    let source = Source::new(&wire);
    let resolver = ModelNames::default();
    let lookup = Registry;
    let openai = Configs {
        events: events.clone(),
        supported: None,
    };
    let strict = Configs {
        events: events.clone(),
        supported: Some(vec![String::from("instructions")]),
    };
    let variables = env_vars(&[("OPENAI_API_KEY", "sk-provider")]);
    let permissive_deps = CompactResponsesDeps {
        resolver: &resolver,
        lookup: &lookup,
        configs: &openai,
        source: &source,
        environment: &variables,
        drop_params: false,
    };
    let strict_deps = CompactResponsesDeps {
        resolver: &resolver,
        lookup: &lookup,
        configs: &strict,
        source: &source,
        environment: &variables,
        drop_params: false,
    };
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
    compact_responses(&permissive_deps, &full, &mut log)
        .await
        .expect("the supported options reach the provider");

    let mut rejected = call("openai/gpt-4o", text_input("hello"));
    rejected.kwargs = kwargs(r#"{"temperature":0.2}"#);
    let Err(first_error) = compact_responses(&strict_deps, &rejected, &mut log).await else {
        panic!("an unsupported parameter must be rejected before the request");
    };
    let first_failure = first_error.sdk_failure("openai", "gpt-4o", &harness.redactor);
    assert_eq!(first_failure.class, "UnsupportedParamsError");
    assert_eq!(first_failure.status, UNSUPPORTED_PARAMS_STATUS);
    assert!(first_failure.kind.is_none());
    assert!(first_failure.param.is_none());
    assert!(first_failure.code.is_none());
    assert_eq!(wire.requests().len(), 1);

    let mut dropping = call("openai/gpt-4o", text_input("hello"));
    dropping.instructions = Some(String::from("short"));
    dropping.kwargs = kwargs(r#"{"temperature":0.2,"drop_params":true}"#);
    compact_responses(&strict_deps, &dropping, &mut log)
        .await
        .expect("drop_params true drops the unsupported parameter");

    let mut text_dropping = call("openai/gpt-4o", text_input("hello"));
    text_dropping.kwargs = kwargs(r#"{"temperature":0.2,"drop_params":"false"}"#);
    let Err(second_error) = compact_responses(&strict_deps, &text_dropping, &mut log).await else {
        panic!("the text spelling false must not drop");
    };
    assert!(matches!(second_error, CompactResponsesError::Param(_)));

    let mut bypass = call("openai/gpt-4o", text_input("hello"));
    bypass.kwargs = kwargs(r#"{"temperature":0.2,"allowed_openai_params":["temperature"]}"#);
    let Err(third_error) = compact_responses(&strict_deps, &bypass, &mut log).await else {
        panic!("compact never reads the caller's allowed_openai_params");
    };
    assert!(matches!(third_error, CompactResponsesError::Param(_)));

    let mut nulled = call("openai/gpt-4o", text_input("hello"));
    nulled.kwargs = kwargs(r#"{"temperature":null}"#);
    compact_responses(&permissive_deps, &nulled, &mut log)
        .await
        .expect("a null optional value is omitted");

    let mut unknown = call("openai/gpt-4o", text_input("hello"));
    unknown.kwargs = kwargs(r#"{"top_k":5,"trace":"t"}"#);
    compact_responses(&permissive_deps, &unknown, &mut log)
        .await
        .expect("unknown kwargs are filtered");

    let requests = wire.requests();
    assert_eq!(requests.len(), 4);
    assert_eq!(
        body_json(&requests[0]),
        json(r#"{"model":"gpt-4o","input":"hello","temperature":0.2,"instructions":"short"}"#)
    );
    assert_eq!(
        body_json(&requests[1]),
        json(r#"{"model":"gpt-4o","input":"hello","instructions":"short"}"#)
    );
    assert_eq!(
        body_json(&requests[2]),
        json(r#"{"model":"gpt-4o","input":"hello"}"#)
    );
    assert_eq!(
        body_json(&requests[3]),
        json(r#"{"model":"gpt-4o","input":"hello"}"#)
    );
    assert_eq!(requests[0].url.query(), None);
}

#[tokio::test]
async fn compact_timeout_and_residency() {
    let events = Events::new();
    let mut harness = Harness::new();
    let wire = Wire::new(&events);
    wire.repeat(200, COMPACT_OK, 13);
    let source = Source::new(&wire);
    let resolver = ModelNames::default();
    let lookup = Registry;
    let configs = Configs {
        events: events.clone(),
        supported: None,
    };
    let variables = env_vars(&[("OPENAI_API_KEY", "sk-provider")]);
    let deps = CompactResponsesDeps {
        resolver: &resolver,
        lookup: &lookup,
        configs: &configs,
        source: &source,
        environment: &variables,
        drop_params: false,
    };
    let longer = env_vars(&[("OPENAI_API_KEY", "sk-provider"), ("REQUEST_TIMEOUT", "17")]);
    let longer_deps = CompactResponsesDeps {
        resolver: &resolver,
        lookup: &lookup,
        configs: &configs,
        source: &source,
        environment: &longer,
        drop_params: false,
    };
    let broken = env_vars(&[
        ("OPENAI_API_KEY", "sk-provider"),
        ("REQUEST_TIMEOUT", "soon"),
    ]);
    let broken_deps = CompactResponsesDeps {
        resolver: &resolver,
        lookup: &lookup,
        configs: &configs,
        source: &source,
        environment: &broken,
        drop_params: false,
    };
    let prepost = base_env(&harness.redactor, &harness.sink, harness.now);
    let mut log = LogContext {
        state: &mut harness.state,
        env: &prepost,
    };
    let mut compacting = call("openai/gpt-4o", text_input("hello"));
    compact_responses(&deps, &compacting, &mut log)
        .await
        .expect("the default timeout call succeeds");
    compacting.timeout = Some(0.0);
    compact_responses(&deps, &compacting, &mut log)
        .await
        .expect("the zero timeout call succeeds");
    compacting.timeout = Some(7.0);
    compact_responses(&deps, &compacting, &mut log)
        .await
        .expect("the explicit timeout call succeeds");
    compacting.timeout = None;
    compact_responses(&longer_deps, &compacting, &mut log)
        .await
        .expect("the configured timeout call succeeds");
    compacting.timeout = Some(0.0);
    compact_responses(&longer_deps, &compacting, &mut log)
        .await
        .expect("the configured zero timeout call succeeds");
    let requests = wire.requests();
    assert_eq!(requests[0].timeout, Some(Duration::from_secs(6000)));
    assert_eq!(requests[1].timeout, Some(Duration::from_secs(6000)));
    assert_eq!(requests[2].timeout, Some(Duration::from_secs(7)));
    assert_eq!(requests[3].timeout, Some(Duration::from_secs(17)));
    assert_eq!(requests[4].timeout, Some(Duration::from_secs(17)));

    let Err(timeout_error) = compact_responses(&broken_deps, &compacting, &mut log).await else {
        panic!("an unreadable REQUEST_TIMEOUT fails the call");
    };
    assert!(matches!(timeout_error, CompactResponsesError::Timeout(_)));

    let mut regional = call("openai/gpt-4o", text_input("hello"));
    regional.api_base = Some(String::from("https://eu.api.openai.com/v1"));
    compact_responses(&deps, &regional, &mut log)
        .await
        .expect("the eu host call succeeds");
    assert_eq!(
        log.state.litellm_params.get("data_residency"),
        Some(&Json::Text(String::from("eu")))
    );
    regional.api_base = Some(String::from("https://us.api.openai.com/v1"));
    compact_responses(&deps, &regional, &mut log)
        .await
        .expect("the us host call succeeds");
    assert_eq!(
        log.state.litellm_params.get("data_residency"),
        Some(&Json::Text(String::from("us")))
    );
    regional.api_base = Some(String::from("https://api.openai.com/v1"));
    compact_responses(&deps, &regional, &mut log)
        .await
        .expect("the global host call succeeds");
    assert_eq!(
        log.state.litellm_params.get("data_residency"),
        Some(&Json::Null)
    );

    let mut handoff = call("openai/gpt-4o", text_input("hello"));
    handoff.api_base = Some(String::from("https://eu.api.openai.com/v1"));
    handoff.instructions = Some(String::from("short"));
    handoff.kwargs = kwargs(r#"{"litellm_call_id":"call-a"}"#);
    assert!(!events.recorded().contains(&String::from("off_loop")));
    acompact_responses(&deps, &handoff, &mut log)
        .await
        .expect("the async handoff call succeeds");
    assert!(events.recorded().contains(&String::from("off_loop")));
    assert_eq!(
        log.state.litellm_params.get("litellm_call_id"),
        Some(&Json::Text(String::from("call-a")))
    );
    assert_eq!(
        log.state.optional_params.get("instructions"),
        Some(&Json::Text(String::from("short")))
    );
    assert_eq!(
        log.state.litellm_params.get("input"),
        Some(&Json::Text(String::from("hello")))
    );
    let seen = resolver.seen();
    let preliminary = seen
        .iter()
        .filter(|resolution| resolution.model == "openai/gpt-4o")
        .collect::<Vec<_>>();
    assert!(
        preliminary[preliminary.len() - 2].api_base.is_none(),
        "the async wrapper's preliminary lookup reads no base"
    );
    assert_eq!(
        preliminary[preliminary.len() - 1].api_base,
        Some(String::from("https://eu.api.openai.com/v1"))
    );

    let resolved_before = resolver.seen().len();
    let mut explicit = call("openai/gpt-4o", text_input("hello"));
    explicit.custom_llm_provider = Some(String::from("openai"));
    acompact_responses(&deps, &explicit, &mut log)
        .await
        .expect("the async call with an explicit provider succeeds");
    let explicit_seen = resolver.seen();
    assert_eq!(explicit_seen.len(), resolved_before + 1);
    assert_eq!(
        explicit_seen[resolved_before]
            .custom_llm_provider
            .as_deref(),
        Some("openai")
    );

    let off_loop_before = events
        .recorded()
        .iter()
        .filter(|event| event.as_str() == "off_loop")
        .count();
    let before = wire.requests().len();
    let mut marked_off = call("openai/gpt-4o", text_input("hello"));
    marked_off.acompact_responses = Some(Json::Bool(false));
    compact_responses(&deps, &marked_off, &mut log)
        .await
        .expect("the false marker call succeeds");
    let mut marked_text = call("openai/gpt-4o", text_input("hello"));
    marked_text.acompact_responses = Some(Json::Text(String::from("true")));
    compact_responses(&deps, &marked_text, &mut log)
        .await
        .expect("the text marker call succeeds");
    let mut marked_on = call("openai/gpt-4o", text_input("hello"));
    marked_on.acompact_responses = Some(Json::Bool(true));
    compact_responses(&deps, &marked_on, &mut log)
        .await
        .expect("the true marker call succeeds");
    assert_eq!(wire.requests().len(), before + 3);
    assert_eq!(source.created().len(), before + 3);
    let off_loop_after = events
        .recorded()
        .iter()
        .filter(|event| event.as_str() == "off_loop")
        .count();
    assert_eq!(
        off_loop_after,
        off_loop_before + 1,
        "only the boolean true marker selects the async path"
    );
}

#[tokio::test]
async fn compact_native_support_required() {
    let events = Events::new();
    let mut harness = Harness::new();
    let wire = Wire::new(&events);
    wire.queue(
        401,
        r#"{"error":{"message":"bad key","type":"invalid_api_key","param":null,"code":"invalid_api_key"}}"#,
    );
    let source = Source::new(&wire);
    let resolver = ModelNames::default();
    let lookup = Registry;
    let configs = Configs {
        events: events.clone(),
        supported: None,
    };
    let variables = env_vars(&[("OPENAI_API_KEY", "sk-provider")]);
    let deps = CompactResponsesDeps {
        resolver: &resolver,
        lookup: &lookup,
        configs: &configs,
        source: &source,
        environment: &variables,
        drop_params: false,
    };
    let prepost = base_env(&harness.redactor, &harness.sink, harness.now);
    let mut log = LogContext {
        state: &mut harness.state,
        env: &prepost,
    };
    let Err(unsupported) = compact_responses(
        &deps,
        &call("anthropic/claude-sonnet-4", text_input("hello")),
        &mut log,
    )
    .await
    else {
        panic!("a provider without a native responses config must fail");
    };
    assert_eq!(
        unsupported.to_string(),
        "COMPACT responses is not supported for anthropic"
    );
    let unsupported_failure =
        unsupported.sdk_failure("anthropic", "claude-sonnet-4", &harness.redactor);
    assert_eq!(unsupported_failure.class, "APIConnectionError");
    assert_eq!(unsupported_failure.status, 500);
    assert!(unsupported_failure.kind.is_none());
    assert!(unsupported_failure.param.is_none());
    assert!(unsupported_failure.code.is_none());
    assert!(wire.requests().is_empty());

    let Err(absent_provider) =
        compact_responses(&deps, &call("none/gpt-4o", text_input("hello")), &mut log).await
    else {
        panic!("a missing provider must fail");
    };
    assert_eq!(
        absent_provider.to_string(),
        "custom_llm_provider is required but passed as None"
    );
    let absent_failure = absent_provider.sdk_failure("", "gpt-4o", &harness.redactor);
    assert_eq!(absent_failure.class, "APIConnectionError");
    assert_eq!(absent_failure.status, 500);
    assert!(wire.requests().is_empty());

    let Err(rejected_key) =
        compact_responses(&deps, &call("openai/gpt-4o", text_input("hello")), &mut log).await
    else {
        panic!("the mock 401 must fail the call");
    };
    let key_failure = rejected_key.sdk_failure("openai", "gpt-4o", &harness.redactor);
    assert_eq!(key_failure.class, "AuthenticationError");
    assert_eq!(key_failure.status, 401);
    assert!(key_failure.kind.is_none());
    assert!(key_failure.param.is_none());
    assert!(key_failure.code.is_none());
    assert!(!key_failure.message.is_empty());
    assert_eq!(wire.requests().len(), 1);
    assert_eq!(
        source.created().len(),
        1,
        "only the provider call creates a client"
    );
}

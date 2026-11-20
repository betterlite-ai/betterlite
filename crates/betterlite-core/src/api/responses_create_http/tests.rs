#![cfg(test)]

mod support;

use std::sync::atomic::Ordering;
use std::time::Duration;

use betterlite_models::decisions::Json as ModelJson;
use betterlite_openai::response_input::ResponsesInput;
use betterlite_prompts::arguments::json::{Json as PromptJson, Object};
use http::Method;
use indexmap::IndexMap;

use self::support::{
    Answer, Config, Dispatcher, NO_CALLBACKS, NoResolver, RESP_A, Reply, Source, Wire,
    agentic_callback, body_map, execute, headers, hooks, json, models, object, params,
    pre_hook_callback, text_input,
};
use crate::api::responses_compact_http::error::Stage as FailStage;
use crate::api::responses_compact_http::service::{ClientKind, SharedSession, SuppliedClient};
use crate::api::responses_create_http::config::CreateParams;
use crate::api::responses_create_http::error::{CreateError, Stage as ErrorStage};
use crate::api::responses_create_http::service::{CreateCall, CreateHooks, CreateOutcome};
use crate::api::responses_hook_policy::callbacks::{CallbackLists, CallbackValue};
use crate::api::responses_stream_driver::source::PayloadSource;

fn create_call<'call>(
    input: &'call ResponsesInput,
    optional_params: &'call Object,
    call_params: &'call CreateParams,
) -> CreateCall<'call> {
    CreateCall {
        model: "gpt-4o",
        input,
        optional_params,
        custom_llm_provider: "openai",
        params: call_params,
        extra_headers: None,
        extra_body: None,
        timeout: Some(7.0_f64),
        fake_stream: None,
        litellm_metadata: None,
        shared_session: None,
    }
}

fn read_value(outcome: &CreateOutcome) -> PromptJson {
    match outcome {
        CreateOutcome::Value(value) => json(value.get()),
        CreateOutcome::Live(_) => panic!("the call returns a stream handoff"),
        CreateOutcome::Synthetic(_) => panic!("the call returns synthetic events"),
    }
}

#[tokio::test]
async fn create_transform_overlay_order() {
    let globals = [CallbackValue::Object(pre_hook_callback())];
    let dispatcher = Dispatcher::patching(IndexMap::from([
        (String::from("model"), models("\"gpt-4o-mini\"")),
        (String::from("api_key"), models("\"sk-hooked\"")),
    ]));
    let resolver = NoResolver;
    let config = Config::default();
    let input = text_input("hello");
    let optional = object(
        r#"{"temperature":0.2,"extra_headers":{"Authorization":"Bearer optional","X-Option":"a"}}"#,
    );
    let extra_headers = headers(&[("Authorization", "Bearer explicit"), ("X-Trace", "b")]);
    let extra_body = object(r#"{"temperature":0.8}"#);
    let call_params = params(r#"{"api_base":"http://mock/v1","api_key":"sk-base"}"#);
    let wire = Wire::new(vec![Answer::Json(Reply::json(200, RESP_A))]);
    let source = Source::with(wire.clone());
    let mut call = create_call(&input, &optional, &call_params);
    call.extra_headers = Some(&extra_headers);
    call.extra_body = Some(&extra_body);
    let callbacks = CallbackLists {
        global: &globals,
        dynamic: &[],
    };
    let create_hooks: CreateHooks<'_, Dispatcher> = hooks(callbacks, &dispatcher, &resolver);
    let outcome = execute(
        &config,
        &source,
        None,
        ClientKind::Sync,
        &call,
        &create_hooks,
    )
    .await
    .expect("the sync call");

    let sent = wire.requests();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].method, Method::POST);
    assert_eq!(sent[0].url.as_str(), "http://mock/v1/responses");
    assert_eq!(sent[0].timeout, Duration::from_secs(7));
    assert_eq!(sent[0].header("authorization"), Some("Bearer explicit"));
    assert_eq!(sent[0].header("x-option"), Some("a"));
    assert_eq!(sent[0].header("x-trace"), Some("b"));
    assert_eq!(sent[0].header("x-signed"), Some("yes"));
    let fields = body_map(&sent[0].body_json()).clone();
    assert_eq!(fields.get("model"), Some(&json("\"gpt-4o-mini\"")));
    assert_eq!(fields.get("temperature"), Some(&json("0.8")));
    assert_eq!(fields.get("input"), Some(&json("\"hello\"")));
    assert!(!fields.contains_key("extra_headers"));
    assert!(!fields.contains_key("timeout"));
    assert!(!fields.contains_key("litellm_params"));
    assert!(!fields.contains_key("request_context"));
    assert_eq!(dispatcher.calls(), 1);

    let validated = config.validated_headers();
    assert_eq!(validated.len(), 1);
    assert_eq!(
        validated[0].get("Authorization").map(String::as_str),
        Some("Bearer optional")
    );
    assert_eq!(validated[0].get("X-Option").map(String::as_str), Some("a"));
    let signed = config.recorded_sign();
    assert_eq!(signed.model, "gpt-4o-mini");
    assert_eq!(signed.api_key.as_deref(), Some("sk-hooked"));
    assert!(!signed.stream);
    assert_eq!(
        signed.headers.get("Authorization").map(String::as_str),
        Some("Bearer explicit")
    );
    assert_eq!(signed.data.get("temperature"), Some(&json("0.8")));
    assert_eq!(
        body_map(&read_value(&outcome)).get("id"),
        Some(&json("\"resp_a\""))
    );

    // async never reruns the sync deployment hook, so the original model and key stay
    let async_dispatcher = Dispatcher::patching(IndexMap::from([(
        String::from("model"),
        models("\"gpt-4o-mini\""),
    )]));
    let async_wire = Wire::new(vec![Answer::Json(Reply::json(200, RESP_A))]);
    let async_source = Source::with(async_wire.clone());
    let async_hooks = hooks(callbacks, &async_dispatcher, &resolver);
    execute(
        &config,
        &async_source,
        None,
        ClientKind::Asynchronous,
        &call,
        &async_hooks,
    )
    .await
    .expect("the async call");
    assert_eq!(async_dispatcher.calls(), 0);
    let async_sent = async_wire.requests();
    let async_body = body_map(&async_sent[0].body_json()).clone();
    assert_eq!(async_body.get("model"), Some(&json("\"gpt-4o\"")));
    assert_eq!(
        async_sent[0].header("authorization"),
        Some("Bearer explicit")
    );
    let async_signed = config.recorded_sign();
    assert_eq!(async_signed.model, "gpt-4o");
    assert_eq!(async_signed.api_key.as_deref(), Some("sk-base"));
}

#[tokio::test]
async fn create_stream_and_fake_selection() {
    let config = Config::default();
    let input = text_input("hello");
    let call_params = params(r#"{"api_base":"http://mock/v1"}"#);
    let dispatcher = Dispatcher::quiet();
    let resolver = NoResolver;
    let metadata = IndexMap::from([(String::from("trace"), models("\"t\""))]);

    // optional stream=false with extra_body.stream=true still selects a streaming call
    let live_wire = Wire::new(vec![Answer::Sse(vec![
        String::from("hello"),
        String::from("[DONE]"),
    ])]);
    let live_source = Source::with(live_wire.clone());
    let live_optional = object(r#"{"stream":false}"#);
    let live_body = object(r#"{"stream":true}"#);
    let mut live_call = create_call(&input, &live_optional, &call_params);
    live_call.extra_body = Some(&live_body);
    live_call.litellm_metadata = Some(&metadata);
    let live_hooks = hooks(NO_CALLBACKS, &dispatcher, &resolver);
    let live_outcome = execute(
        &config,
        &live_source,
        None,
        ClientKind::Asynchronous,
        &live_call,
        &live_hooks,
    )
    .await
    .expect("the live call");
    assert!(live_wire.requests()[0].stream);
    assert_eq!(
        body_map(&live_wire.requests()[0].body_json()).get("stream"),
        Some(&json("true"))
    );
    let CreateOutcome::Live(mut live) = live_outcome else {
        panic!("a streaming send returns the live iterator handoff")
    };
    let mut payloads = Vec::new();
    while let Some(item) = live.source.next_payload().await {
        match item {
            Ok(bytes) => payloads.push(String::from_utf8(bytes).expect("the payload is UTF-8")),
            Err(error) => panic!("the payload reads: {error:?}"),
        }
    }
    assert_eq!(
        payloads,
        vec![String::from("hello"), String::from("[DONE]")]
    );
    assert_eq!(live.setup.model, "gpt-4o");
    assert_eq!(live.setup.custom_llm_provider, "openai");
    assert_eq!(live.setup.call_type.as_deref(), Some("responses"));
    assert_eq!(
        live.setup
            .litellm_metadata
            .as_ref()
            .and_then(|known| known.get("trace")),
        Some(&models("\"t\""))
    );
    let context = live.setup.request_data.expect("the request context");
    assert_eq!(context.get("input"), Some(&models("\"hello\"")));
    assert_eq!(context.get("stream"), Some(&models("false")));
    assert!(matches!(
        context.get("litellm_params"),
        Some(ModelJson::Object(_))
    ));

    // fake_stream=true removes body.stream and sends a buffered request, then returns synthetic events
    let fake_wire = Wire::new(vec![Answer::Json(Reply::json(200, RESP_A))]);
    let fake_source = Source::with(fake_wire.clone());
    let fake_optional = object(r#"{"stream":true}"#);
    let fake_body = object(r#"{"stream":false}"#);
    let fake = models("true");
    let mut fake_call = create_call(&input, &fake_optional, &call_params);
    fake_call.extra_body = Some(&fake_body);
    fake_call.fake_stream = Some(&fake);
    let fake_hooks = hooks(NO_CALLBACKS, &dispatcher, &resolver);
    let fake_outcome = execute(
        &config,
        &fake_source,
        None,
        ClientKind::Sync,
        &fake_call,
        &fake_hooks,
    )
    .await
    .expect("the fake call");
    assert!(!fake_wire.requests()[0].stream);
    assert!(
        body_map(&fake_wire.requests()[0].body_json())
            .get("stream")
            .is_none()
    );
    let CreateOutcome::Synthetic(synthetic) = fake_outcome else {
        panic!("a prepared fake stream returns the synthetic handoff")
    };
    assert_eq!(synthetic.response.id, "resp_a");
    assert_eq!(synthetic.setup.call_type.as_deref(), Some("responses"));
    assert_eq!(
        synthetic
            .setup
            .request_data
            .as_ref()
            .and_then(|context| context.get("input")),
        Some(&models("\"hello\""))
    );

    // fake_stream=1 does not satisfy the `is True` identity guard, so the call streams
    let one_wire = Wire::new(vec![Answer::Sse(vec![String::from("[DONE]")])]);
    let one_source = Source::with(one_wire.clone());
    let one_optional = object(r#"{"stream":true}"#);
    let one = models("1");
    let mut one_call = create_call(&input, &one_optional, &call_params);
    one_call.fake_stream = Some(&one);
    let one_hooks = hooks(NO_CALLBACKS, &dispatcher, &resolver);
    let one_outcome = execute(
        &config,
        &one_source,
        None,
        ClientKind::Sync,
        &one_call,
        &one_hooks,
    )
    .await
    .expect("the identity-guard call");
    assert!(matches!(one_outcome, CreateOutcome::Live(_)));
    assert!(one_wire.requests()[0].stream);

    // a nonstream call with fake_stream ignores both the flag and the stream key
    let plain_wire = Wire::new(vec![Answer::Json(Reply::json(200, RESP_A))]);
    let plain_source = Source::with(plain_wire.clone());
    let empty = Object::new();
    let mut plain_call = create_call(&input, &empty, &call_params);
    plain_call.fake_stream = Some(&fake);
    let plain_hooks = hooks(NO_CALLBACKS, &dispatcher, &resolver);
    let plain_outcome = execute(
        &config,
        &plain_source,
        None,
        ClientKind::Sync,
        &plain_call,
        &plain_hooks,
    )
    .await
    .expect("the plain call");
    assert_eq!(
        body_map(&read_value(&plain_outcome)).get("id"),
        Some(&json("\"resp_a\""))
    );
    assert!(!plain_wire.requests()[0].stream);
    assert!(
        body_map(&plain_wire.requests()[0].body_json())
            .get("stream")
            .is_none()
    );
}

#[tokio::test]
async fn create_sign_timeout_and_client() {
    let dispatcher = Dispatcher::quiet();
    let resolver = NoResolver;
    let input = text_input("hello");
    let call_params = params(r#"{"api_base":"http://mock/v1","ssl_verify":false}"#);
    let cases = [
        (Some(7.0_f64), r#"{"timeout":"9"}"#, Duration::from_secs(7)),
        (Some(0.0_f64), r#"{"timeout":"9"}"#, Duration::from_secs(9)),
        (None, r#"{}"#, Duration::ZERO),
        (None, r#"{"timeout":3.5}"#, Duration::from_millis(3500)),
        (None, r#"{"timeout":"inf"}"#, Duration::MAX),
    ];
    for (explicit, optional_text, expected) in cases {
        let config = Config::default();
        let optional = object(optional_text);
        let wire = Wire::new(vec![Answer::Json(Reply::json(200, RESP_A))]);
        let source = Source::with(wire.clone());
        let mut call = create_call(&input, &optional, &call_params);
        call.timeout = explicit;
        let create_hooks = hooks(NO_CALLBACKS, &dispatcher, &resolver);
        execute(
            &config,
            &source,
            None,
            ClientKind::Sync,
            &call,
            &create_hooks,
        )
        .await
        .expect("the timed call");
        assert_eq!(wire.requests()[0].timeout, expected, "{optional_text}");
    }

    // a timeout that float() rejects becomes B2402's synthesized 500 before any HTTP request
    let bad_optional = object(r#"{"timeout":"abc"}"#);
    let bad_wire = Wire::new(Vec::new());
    let bad_source = Source::with(bad_wire.clone());
    let bad_config = Config::default();
    let mut bad_call = create_call(&input, &bad_optional, &call_params);
    bad_call.timeout = None;
    let bad_hooks = hooks(NO_CALLBACKS, &dispatcher, &resolver);
    let error = match execute(
        &bad_config,
        &bad_source,
        None,
        ClientKind::Sync,
        &bad_call,
        &bad_hooks,
    )
    .await
    {
        Ok(_) => panic!("the bad timeout fails"),
        Err(error) => error,
    };
    let CreateError::Provider(provider) = error else {
        panic!("the timeout conversion failure is a provider error")
    };
    assert_eq!(provider.status_code, 500);
    assert!(provider.status_code_is_synthesized);
    assert_eq!(provider.message, "could not convert string to float: 'abc'");
    assert!(bad_wire.requests().is_empty());

    // signing sees the post-overlay, post-fake-prep body; empty signed bytes stay the whole body
    let signed_config = Config {
        signed_body: Some(Vec::new()),
        ..Config::default()
    };
    let signed_optional = object(r#"{"stream":true}"#);
    let signed_body = object(r#"{"temperature":0.8}"#);
    let fake = models("true");
    let signed_wire = Wire::new(vec![Answer::Json(Reply::json(200, RESP_A))]);
    let signed_source = Source::with(signed_wire.clone());
    let mut signed_call = create_call(&input, &signed_optional, &call_params);
    signed_call.extra_body = Some(&signed_body);
    signed_call.fake_stream = Some(&fake);
    signed_call.timeout = Some(1.0_f64);
    let signed_hooks = hooks(NO_CALLBACKS, &dispatcher, &resolver);
    execute(
        &signed_config,
        &signed_source,
        None,
        ClientKind::Sync,
        &signed_call,
        &signed_hooks,
    )
    .await
    .expect("the signed call");
    let recorded = signed_config.recorded_sign();
    assert!(!recorded.stream);
    assert!(matches!(recorded.fake_stream, Some(ModelJson::Bool(true))));
    assert_eq!(recorded.data.get("temperature"), Some(&json("0.8")));
    assert!(recorded.data.get("stream").is_none());
    assert_eq!(recorded.api_base, "http://mock/v1/responses");
    assert!(signed_wire.requests()[0].body.is_empty());

    // a supplied client of the matching kind is reused; the wrong kind selects the pool
    let empty = Object::new();
    let client_config = Config::default();
    let reuse_wire = Wire::new(vec![Answer::Json(Reply::json(200, RESP_A))]);
    let reuse_supplied = SuppliedClient {
        kind: ClientKind::Sync,
        transport: &reuse_wire,
    };
    let reuse_pool = Wire::new(Vec::new());
    let reuse_source = Source::with(reuse_pool.clone());
    let mut client_call = create_call(&input, &empty, &call_params);
    client_call.timeout = Some(1.0_f64);
    let client_hooks = hooks(NO_CALLBACKS, &dispatcher, &resolver);
    execute(
        &client_config,
        &reuse_source,
        Some(reuse_supplied),
        ClientKind::Sync,
        &client_call,
        &client_hooks,
    )
    .await
    .expect("the supplied client call");
    assert_eq!(reuse_wire.requests().len(), 1);
    assert!(reuse_source.creations().is_empty());

    let wrong_wire = Wire::new(Vec::new());
    let wrong_supplied = SuppliedClient {
        kind: ClientKind::Sync,
        transport: &wrong_wire,
    };
    let wrong_pool = Wire::new(vec![Answer::Json(Reply::json(200, RESP_A))]);
    let wrong_source = Source::with(wrong_pool.clone());
    execute(
        &client_config,
        &wrong_source,
        Some(wrong_supplied),
        ClientKind::Asynchronous,
        &client_call,
        &client_hooks,
    )
    .await
    .expect("the wrong-kind call");
    assert!(wrong_wire.requests().is_empty());
    assert_eq!(wrong_pool.requests().len(), 1);

    // async forwards ssl_verify and shared_session to the pool and signs off the loop; sync forwards neither
    let async_wire = Wire::new(vec![Answer::Json(Reply::json(200, RESP_A))]);
    let async_source = Source::with(async_wire.clone());
    let async_call = CreateCall {
        shared_session: Some(SharedSession(9)),
        ..client_call
    };
    let async_config = Config::default();
    execute(
        &async_config,
        &async_source,
        None,
        ClientKind::Asynchronous,
        &async_call,
        &client_hooks,
    )
    .await
    .expect("the async client call");
    assert_eq!(
        async_source.creations(),
        vec![(Some(false), Some(SharedSession(9)))]
    );
    assert!(async_config.off_loop_sign.load(Ordering::SeqCst));
    assert!(async_config.off_loop_transform.load(Ordering::SeqCst));

    let sync_wire = Wire::new(vec![Answer::Json(Reply::json(200, RESP_A))]);
    let sync_source = Source::with(sync_wire.clone());
    let mut sync_call = create_call(&input, &empty, &call_params);
    sync_call.timeout = Some(1.0_f64);
    sync_call.shared_session = Some(SharedSession(9));
    let sync_config = Config::default();
    execute(
        &sync_config,
        &sync_source,
        None,
        ClientKind::Sync,
        &sync_call,
        &client_hooks,
    )
    .await
    .expect("the sync client call");
    assert_eq!(
        sync_source.creations(),
        vec![(Some(false), None::<SharedSession>)]
    );
    assert!(!sync_config.off_loop_sign.load(Ordering::SeqCst));
    assert!(!sync_config.off_loop_transform.load(Ordering::SeqCst));
}

#[tokio::test]
async fn create_agentic_result_wrapping() {
    let hook_response = r#"{"id":"resp_hook","created_at":2,"object":"response","status":"completed","model":"gpt-4o","output":[]}"#;
    let input = text_input("hello");
    let empty = Object::new();
    let dispatcher = Dispatcher::quiet();
    let resolver = NoResolver;

    // every converted-stream flag wraps the hook's result with no request context or metadata
    for flag in [
        "_websearch_interception_converted_stream",
        "_compression_interception_converted_stream",
        "_headroom_interception_converted_stream",
        "_code_interpreter_interception_converted_stream",
    ] {
        let globals = [CallbackValue::Object(agentic_callback(Some(hook_response)))];
        let callbacks = CallbackLists {
            global: &globals,
            dynamic: &[],
        };
        let create_hooks = hooks(callbacks, &dispatcher, &resolver);
        let flag_params = params(&format!(r#"{{"api_base":"http://mock/v1","{flag}":true}}"#));
        let wire = Wire::new(vec![Answer::Json(Reply::json(200, RESP_A))]);
        let source = Source::with(wire.clone());
        let call = create_call(&input, &empty, &flag_params);
        let config = Config::default();
        let outcome = execute(
            &config,
            &source,
            None,
            ClientKind::Asynchronous,
            &call,
            &create_hooks,
        )
        .await
        .expect("the agentic call");
        let CreateOutcome::Synthetic(synthetic) = outcome else {
            panic!("{flag} wraps the result as synthetic events")
        };
        assert_eq!(synthetic.response.id, "resp_hook", "{flag}");
        assert_eq!(synthetic.setup.model, "gpt-4o", "{flag}");
        assert!(synthetic.setup.request_data.is_none(), "{flag}");
        assert!(synthetic.setup.litellm_metadata.is_none(), "{flag}");
        assert!(synthetic.setup.call_type.is_none(), "{flag}");
    }

    // a nonzero depth prevents the wrap, and the hook result is returned as plain JSON
    let depth_globals = [CallbackValue::Object(agentic_callback(Some(hook_response)))];
    let depth_callbacks = CallbackLists {
        global: &depth_globals,
        dynamic: &[],
    };
    let depth_hooks = hooks(depth_callbacks, &dispatcher, &resolver);
    let depth_params = params(
        r#"{"api_base":"http://mock/v1","_websearch_interception_converted_stream":true,"_agentic_loop_depth":1}"#,
    );
    let depth_wire = Wire::new(vec![Answer::Json(Reply::json(200, RESP_A))]);
    let depth_source = Source::with(depth_wire.clone());
    let depth_call = create_call(&input, &empty, &depth_params);
    let depth_config = Config::default();
    let depth_outcome = execute(
        &depth_config,
        &depth_source,
        None,
        ClientKind::Sync,
        &depth_call,
        &depth_hooks,
    )
    .await
    .expect("the depth call");
    assert_eq!(
        body_map(&read_value(&depth_outcome)).get("id"),
        Some(&json("\"resp_hook\""))
    );

    // a null hook result keeps the initial response; with a converted flag it still wraps
    let null_globals = [CallbackValue::Object(agentic_callback(None))];
    let null_callbacks = CallbackLists {
        global: &null_globals,
        dynamic: &[],
    };
    let null_hooks = hooks(null_callbacks, &dispatcher, &resolver);
    let null_params =
        params(r#"{"api_base":"http://mock/v1","_websearch_interception_converted_stream":true}"#);
    let null_wire = Wire::new(vec![Answer::Json(Reply::json(200, RESP_A))]);
    let null_source = Source::with(null_wire.clone());
    let null_call = create_call(&input, &empty, &null_params);
    let null_config = Config::default();
    let null_outcome = execute(
        &null_config,
        &null_source,
        None,
        ClientKind::Asynchronous,
        &null_call,
        &null_hooks,
    )
    .await
    .expect("the null hook call");
    let CreateOutcome::Synthetic(synthetic) = null_outcome else {
        panic!("a null hook result wraps the initial response")
    };
    assert_eq!(synthetic.response.id, "resp_a");

    // a non-null false result replaces the initial response even though it is falsy
    let false_globals = [CallbackValue::Object(agentic_callback(Some("false")))];
    let false_callbacks = CallbackLists {
        global: &false_globals,
        dynamic: &[],
    };
    let false_hooks = hooks(false_callbacks, &dispatcher, &resolver);
    let plain_params = params(r#"{"api_base":"http://mock/v1"}"#);
    let false_wire = Wire::new(vec![Answer::Json(Reply::json(200, RESP_A))]);
    let false_source = Source::with(false_wire.clone());
    let false_call = create_call(&input, &empty, &plain_params);
    let false_config = Config::default();
    let false_outcome = execute(
        &false_config,
        &false_source,
        None,
        ClientKind::Asynchronous,
        &false_call,
        &false_hooks,
    )
    .await
    .expect("the false result call");
    assert_eq!(read_value(&false_outcome), json("false"));

    // a falsy flag value does not wrap even when the agentic result is null
    let falsy_params =
        params(r#"{"api_base":"http://mock/v1","_headroom_interception_converted_stream":""}"#);
    let falsy_wire = Wire::new(vec![Answer::Json(Reply::json(200, RESP_A))]);
    let falsy_source = Source::with(falsy_wire.clone());
    let falsy_call = create_call(&input, &empty, &falsy_params);
    let falsy_config = Config::default();
    let falsy_outcome = execute(
        &falsy_config,
        &falsy_source,
        None,
        ClientKind::Asynchronous,
        &falsy_call,
        &null_hooks,
    )
    .await
    .expect("the falsy flag call");
    assert_eq!(
        body_map(&read_value(&falsy_outcome)).get("id"),
        Some(&json("\"resp_a\""))
    );

    // sync with no agentic hook returns plain JSON even when a converted flag exists; async wraps it
    let quiet_hooks = hooks(NO_CALLBACKS, &dispatcher, &resolver);
    let truthy_params =
        params(r#"{"api_base":"http://mock/v1","_websearch_interception_converted_stream":true}"#);
    let truthy_call = create_call(&input, &empty, &truthy_params);
    let sync_wire = Wire::new(vec![Answer::Json(Reply::json(200, RESP_A))]);
    let sync_source = Source::with(sync_wire.clone());
    let sync_config = Config::default();
    let sync_outcome = execute(
        &sync_config,
        &sync_source,
        None,
        ClientKind::Sync,
        &truthy_call,
        &quiet_hooks,
    )
    .await
    .expect("the sync no-hook call");
    assert_eq!(
        body_map(&read_value(&sync_outcome)).get("id"),
        Some(&json("\"resp_a\""))
    );
    let async_wire = Wire::new(vec![Answer::Json(Reply::json(200, RESP_A))]);
    let async_source = Source::with(async_wire.clone());
    let async_config = Config::default();
    let async_outcome = execute(
        &async_config,
        &async_source,
        None,
        ClientKind::Asynchronous,
        &truthy_call,
        &quiet_hooks,
    )
    .await
    .expect("the async no-hook call");
    let CreateOutcome::Synthetic(wrapped) = async_outcome else {
        panic!("async wraps the initial response when the agentic result is null")
    };
    assert_eq!(wrapped.response.id, "resp_a");
}

#[tokio::test]
async fn create_transport_failure() {
    let dispatcher = Dispatcher::quiet();
    let resolver = NoResolver;
    let input = text_input("hello");
    let empty = Object::new();
    let call_params = params(r#"{"api_base":"http://mock/v1"}"#);
    let create_hooks = hooks(NO_CALLBACKS, &dispatcher, &resolver);
    let default_config = Config::default();
    let call = create_call(&input, &empty, &call_params);

    let error_wire = Wire::new(vec![Answer::Json(
        Reply::json(429, "slow down").with_header("retry-after", "3"),
    )]);
    let error_source = Source::with(error_wire.clone());
    let status_error = match execute(
        &default_config,
        &error_source,
        None,
        ClientKind::Sync,
        &call,
        &create_hooks,
    )
    .await
    {
        Ok(_) => panic!("the 429 fails"),
        Err(error) => error,
    };
    let CreateError::Provider(provider) = status_error else {
        panic!("a 429 is a provider error")
    };
    assert_eq!(provider.status_code, 429);
    assert!(!provider.status_code_is_synthesized);
    assert_eq!(provider.message, "slow down");
    assert_eq!(provider.headers.get("retry-after"), Some(&models("\"3\"")));
    assert_eq!(error_wire.requests().len(), 1);

    // every non-send step fails before the send catch and sends nothing
    for stage in [FailStage::Transform, FailStage::Normalize, FailStage::Sign] {
        let expected = match stage {
            FailStage::Transform => ErrorStage::Transform,
            FailStage::Normalize => ErrorStage::Normalize,
            FailStage::Sign => ErrorStage::Sign,
            FailStage::Url | FailStage::Validate | FailStage::Response => {
                panic!("the loop only fails the transfer steps")
            }
        };
        let config = Config {
            fail: Some(stage),
            ..Config::default()
        };
        let step_wire = Wire::new(Vec::new());
        let step_source = Source::with(step_wire.clone());
        let error = match execute(
            &config,
            &step_source,
            None,
            ClientKind::Sync,
            &call,
            &create_hooks,
        )
        .await
        {
            Ok(_) => panic!("the step fails"),
            Err(error) => error,
        };
        match error {
            CreateError::Step { stage: found, .. } => assert_eq!(found, expected),
            other => panic!("the failure is a step error: {other:?}"),
        }
        assert!(step_wire.requests().is_empty());
    }

    // the response transform runs after the send catch, so the request has already gone out
    let transform_config = Config {
        fail: Some(FailStage::Response),
        ..Config::default()
    };
    let transform_wire = Wire::new(vec![Answer::Json(Reply::json(200, RESP_A))]);
    let transform_source = Source::with(transform_wire.clone());
    let error = match execute(
        &transform_config,
        &transform_source,
        None,
        ClientKind::Sync,
        &call,
        &create_hooks,
    )
    .await
    {
        Ok(_) => panic!("the response transform fails"),
        Err(error) => error,
    };
    match error {
        CreateError::Step {
            stage: ErrorStage::Response,
            ..
        } => {}
        other => panic!("the failure is the response transform: {other:?}"),
    }
    assert_eq!(transform_wire.requests().len(), 1);

    // the optional map updates the request context first and generic params still land last
    let context_optional =
        object(r#"{"stream":true,"input":"override","litellm_params":"ignored"}"#);
    let fake = models("true");
    let context_wire = Wire::new(vec![Answer::Json(Reply::json(200, RESP_A))]);
    let context_source = Source::with(context_wire.clone());
    let mut context_call = create_call(&input, &context_optional, &call_params);
    context_call.fake_stream = Some(&fake);
    let context_config = Config::default();
    let outcome = execute(
        &context_config,
        &context_source,
        None,
        ClientKind::Sync,
        &context_call,
        &create_hooks,
    )
    .await
    .expect("the context call");
    let CreateOutcome::Synthetic(synthetic) = outcome else {
        panic!("the fake call returns synthetic events")
    };
    let context = synthetic.setup.request_data.expect("the request context");
    assert_eq!(context.get("input"), Some(&models("\"override\"")));
    assert_eq!(context.get("stream"), Some(&models("true")));
    assert!(matches!(
        context.get("litellm_params"),
        Some(ModelJson::Object(_))
    ));
}

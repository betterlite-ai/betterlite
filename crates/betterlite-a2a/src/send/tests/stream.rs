use std::collections::BTreeMap;

use betterlite_models::decisions::Json;

use crate::completion_bridge::tests::Source;
use crate::completion_bridge::tests::support::{collect_events, parsed, provider, recorder};
use crate::send::asend_message_streaming;
use crate::send::call::StreamCall;
use crate::send::deps::BridgeDeps;

use super::support::{
    Reply, StreamFixture, details, json, logging_state, message_frame, params_with_provider,
    pricing_params, recorded, recording_dispatch, request, status_frame, with_agent,
};

#[tokio::test]
async fn stream_logging_builds_an_object_when_absent() {
    let (outcome, _seen) = with_agent(
        Reply::Sse(vec![status_frame("working"), message_frame("hi")]),
        |base| async move {
            let mut fixture = StreamFixture::new();
            let completion = recorder();
            let providers = provider();
            let mut source = Source::default();
            let mut bridge = BridgeDeps {
                completion: &completion,
                providers: &providers,
                source: &mut source,
            };
            let (dispatch, calls) = recording_dispatch();
            let call = StreamCall {
                a2a_client: None,
                request: request("hello world"),
                api_base: Some(base),
                litellm_params: pricing_params(),
                agent_id: Some("agent-7".to_owned()),
                metadata: Some(json(r#"{"user_id":"u-1"}"#)),
                proxy_server_request: Some(json(r#"{"path":"/v1/a2a/message"}"#)),
                agent_extra_headers: None,
            };
            let deps = fixture.deps(dispatch);
            let events = asend_message_streaming(call, None, deps, &mut bridge)
                .await
                .expect("the stream starts");
            collect_events(events).await;
            (recorded(&calls), fixture.issued())
        },
    )
    .await;

    let recorded = outcome.0;
    assert_eq!(recorded.len(), 1);
    let logging = &recorded[0].logging;
    assert_eq!(logging.call_type, "asend_message_streaming");
    assert_eq!(logging.model, "a2a_agent/fake-agent");
    assert_eq!(logging.custom_llm_provider.as_deref(), Some("a2a_agent"));
    assert_eq!(logging.litellm_trace_id, "generated-1");
    assert_eq!(outcome.1, vec!["generated-1".to_owned()]);
    let built_params = json(
        r#"{"cost_per_query":3,"input_cost_per_token":0.0001,"metadata":{"user_id":"u-1"},"proxy_server_request":{"path":"/v1/a2a/message"}}"#,
    );
    assert_eq!(Json::Object(logging.litellm_params.clone()), built_params);
    assert_eq!(Json::Object(logging.optional_params.clone()), built_params);
    assert_eq!(
        details(logging, "model"),
        Some(json(r#""a2a_agent/fake-agent""#))
    );
    assert_eq!(
        details(logging, "custom_llm_provider"),
        Some(json(r#""a2a_agent""#))
    );
    assert_eq!(
        details(logging, "call_type"),
        Some(json(r#""asend_message_streaming""#))
    );
    assert_eq!(details(logging, "agent_id"), Some(json(r#""agent-7""#)));
    assert_eq!(
        details(logging, "metadata"),
        Some(json(r#"{"user_id":"u-1"}"#))
    );
    assert_eq!(
        details(logging, "usage"),
        Some(json(
            r#"{"prompt_tokens":2,"completion_tokens":1,"total_tokens":3}"#
        ))
    );
    assert_eq!(details(logging, "stream"), Some(json("false")));
}

#[tokio::test]
async fn stream_logging_reuses_the_given_object() {
    let (outcome, _seen) = with_agent(Reply::Sse(vec![message_frame("hi")]), |base| async move {
        let mut fixture = StreamFixture::new();
        let completion = recorder();
        let providers = provider();
        let mut source = Source::default();
        let mut bridge = BridgeDeps {
            completion: &completion,
            providers: &providers,
            source: &mut source,
        };
        let (dispatch, calls) = recording_dispatch();
        let call = StreamCall {
            a2a_client: None,
            request: request("hello"),
            api_base: Some(base),
            litellm_params: BTreeMap::new(),
            agent_id: Some("agent-7".to_owned()),
            metadata: None,
            proxy_server_request: None,
            agent_extra_headers: None,
        };
        let deps = fixture.deps(dispatch);
        let logging = logging_state(Some("logging-trace"));
        let events = asend_message_streaming(call, Some(logging), deps, &mut bridge)
            .await
            .expect("the stream starts");
        collect_events(events).await;
        (recorded(&calls), fixture.issued())
    })
    .await;

    let recorded = outcome.0;
    assert_eq!(recorded.len(), 1);
    let logging = &recorded[0].logging;
    assert_eq!(logging.call_type, "acompletion");
    assert_eq!(logging.model, "a2a_agent/fake-agent");
    assert_eq!(logging.custom_llm_provider.as_deref(), Some("a2a_agent"));
    assert_eq!(logging.litellm_trace_id, "logging-trace");
    assert!(logging.litellm_params.is_empty());
    assert!(logging.optional_params.is_empty());
    assert_eq!(details(logging, "agent_id"), Some(json(r#""agent-7""#)));
    assert!(outcome.1.is_empty());
}

#[tokio::test]
async fn stream_iterator_yields_the_agent_chunks() {
    let (outcome, _seen) = with_agent(
        Reply::Sse(vec![status_frame("working"), message_frame("hi")]),
        |base| async move {
            let mut fixture = StreamFixture::new();
            let completion = recorder();
            let providers = provider();
            let mut source = Source::default();
            let mut bridge = BridgeDeps {
                completion: &completion,
                providers: &providers,
                source: &mut source,
            };
            let (dispatch, calls) = recording_dispatch();
            let call = StreamCall {
                a2a_client: None,
                request: request("hello world"),
                api_base: Some(base),
                litellm_params: params_with_provider(None),
                agent_id: None,
                metadata: None,
                proxy_server_request: None,
                agent_extra_headers: None,
            };
            let deps = fixture.deps(dispatch);
            let events = asend_message_streaming(call, None, deps, &mut bridge)
                .await
                .expect("the stream starts");
            let collected = collect_events(events).await;
            (collected, recorded(&calls))
        },
    )
    .await;

    let collected = outcome.0;
    assert_eq!(collected.len(), 2);
    let first = collected[0]
        .as_ref()
        .expect("the status chunk is forwarded");
    assert_eq!(first["result"]["kind"], parsed(r#""status-update""#));
    assert_eq!(first["result"]["status"]["state"], parsed(r#""working""#));
    let second = collected[1]
        .as_ref()
        .expect("the message chunk is forwarded");
    assert_eq!(second["result"]["kind"], parsed(r#""message""#));
    assert_eq!(second["result"]["parts"][0]["text"], parsed(r#""hi""#));
    assert_eq!(second["id"], parsed(r#""request-1""#));
    let recorded = outcome.1;
    assert_eq!(recorded.len(), 1);
    assert_eq!(
        details(&recorded[0].logging, "usage"),
        Some(json(
            r#"{"prompt_tokens":2,"completion_tokens":1,"total_tokens":3}"#
        ))
    );
}

#[tokio::test]
async fn stream_iterator_reports_a_stream_error() {
    let (outcome, _seen) = with_agent(
        Reply::Sse(vec!["data: not-json\n\n".to_owned()]),
        |base| async move {
            let mut fixture = StreamFixture::new();
            let completion = recorder();
            let providers = provider();
            let mut source = Source::default();
            let mut bridge = BridgeDeps {
                completion: &completion,
                providers: &providers,
                source: &mut source,
            };
            let (dispatch, calls) = recording_dispatch();
            let call = StreamCall {
                a2a_client: None,
                request: request("hello"),
                api_base: Some(base),
                litellm_params: params_with_provider(None),
                agent_id: None,
                metadata: None,
                proxy_server_request: None,
                agent_extra_headers: None,
            };
            let deps = fixture.deps(dispatch);
            let events = asend_message_streaming(call, None, deps, &mut bridge)
                .await
                .expect("the stream starts");
            let collected = collect_events(events).await;
            (collected, recorded(&calls))
        },
    )
    .await;

    let collected = outcome.0;
    assert_eq!(collected.len(), 1);
    assert!(collected[0].is_err());
    assert!(outcome.1.is_empty());
}

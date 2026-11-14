use std::collections::BTreeMap;

use betterlite_core::agents::json::AgentJson;
use rstest::rstest;

use crate::completion_bridge::A2A_USER_API_KEY_HASH_PARAM;
use crate::completion_bridge::completion::{CompletionChunk, CompletionPort};
use crate::completion_bridge::dispatch::{
    A2AProviderDispatch, BridgeError, Buffered, CompletionError, Streamed, dispatch,
    handle_non_streaming_direct, handle_streaming_direct,
};
use crate::completion_bridge::params::{BridgeCall, ParamsError};
use crate::completion_bridge::tests::Source;
use crate::completion_bridge::tests::support::{
    TestFailure, bridge_call, chunk, collect_events, litellm_params, model_response, parsed,
    provider, provider_calls, recorded_calls, recorder, selected_configs, selector_params,
    send_params, summaries,
};
use crate::provider_selector::A2AProviderConfig;
use crate::watsonx_run::accumulator::MAX_TEXT_BYTES;

#[rstest]
#[case::pydantic_ai(Some("pydantic_ai_agents"), None, A2AProviderConfig::PydanticAI)]
#[case::agentcore(
    Some("bedrock"),
    Some("agentcore/arn"),
    A2AProviderConfig::BedrockAgentCore
)]
#[case::agentcore_in_a_longer_model(
    Some("bedrock"),
    Some("bedrock-with-agentcore-support"),
    A2AProviderConfig::BedrockAgentCore
)]
#[case::langflow(Some("langflow"), Some("flow-1"), A2AProviderConfig::LangFlow)]
#[case::watsonx(
    Some("watsonx_orchestrate"),
    Some("agent"),
    A2AProviderConfig::WatsonxOrchestrate
)]
#[tokio::test]
async fn routes_non_streaming_to_the_selected_config(
    #[case] provider_name: Option<&str>,
    #[case] model: Option<&str>,
    #[case] expected: A2AProviderConfig,
) {
    let params = selector_params(provider_name, model);
    let message = send_params();
    let call = bridge_call("req-1", &message, &params);
    let completion = recorder();
    let providers = provider();
    let mut source = Source::default();
    let result = dispatch::<Buffered, _, _, _>(&call, &completion, &providers, &mut source)
        .await
        .expect("the fixture dispatch succeeds");
    assert_eq!(Some(&result), providers.answer.as_ref().ok());
    assert_eq!(selected_configs(&providers), vec![expected]);
    assert!(recorded_calls(&completion).is_empty());
}

#[rstest]
#[case::unrouted_provider(Some("openai"), Some("gpt-4o"))]
#[case::bedrock_without_agentcore(Some("bedrock"), Some("claude-3-5-sonnet"))]
#[case::no_provider(None, Some("gpt-4o"))]
#[tokio::test]
async fn unrouted_requests_fall_back_to_completion(
    #[case] provider_name: Option<&str>,
    #[case] model: Option<&str>,
) {
    let params = selector_params(provider_name, model);
    let message = send_params();
    let call = bridge_call("req-1", &message, &params);
    let completion = recorder();
    let providers = provider();
    let mut source = Source::default();
    let result = dispatch::<Buffered, _, _, _>(&call, &completion, &providers, &mut source)
        .await
        .expect("the fixture dispatch succeeds");
    assert_eq!(
        result,
        parsed(
            r#"{"jsonrpc":"2.0","id":"req-1","result":{"kind":"message","role":"agent","parts":[{"kind":"text","text":""}],"messageId":"00000000000000000000000000000001"}}"#
        )
    );
    assert!(selected_configs(&providers).is_empty());
    assert_eq!(recorded_calls(&completion).len(), 1);
}

#[tokio::test]
async fn non_streaming_completion_converts_the_model_response() {
    let params = litellm_params(&[
        ("temperature", parsed("0.2")),
        ("model", AgentJson::String("gpt-4o".to_owned())),
    ]);
    let message = send_params();
    let call = bridge_call("req-1", &message, &params);
    let mut completion = recorder();
    completion.answer = Ok(model_response("Hello"));
    let providers = provider();
    let mut source = Source::default();
    let result = dispatch::<Buffered, _, _, _>(&call, &completion, &providers, &mut source)
        .await
        .expect("the fixture dispatch succeeds");
    assert_eq!(
        result,
        parsed(
            r#"{"jsonrpc":"2.0","id":"req-1","result":{"kind":"message","role":"agent","parts":[{"kind":"text","text":"Hello"}],"messageId":"00000000000000000000000000000001"}}"#
        )
    );
    let calls = recorded_calls(&completion);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].model, "gpt-4o");
    assert!(!calls[0].stream);
    assert_eq!(
        calls[0].extra,
        litellm_params(&[("temperature", parsed("0.2"))])
    );
    assert_eq!(
        serde_json::to_value(&calls[0].messages).expect("the messages encode"),
        parsed(r#"[{"role":"user","content":"hello"}]"#)
    );
}

#[tokio::test]
async fn non_streaming_completion_failures_keep_their_source() {
    let params = litellm_params(&[]);
    let message = send_params();
    let call = bridge_call("req-1", &message, &params);
    let mut completion = recorder();
    completion.answer = Err(TestFailure("completion-boom"));
    let providers = provider();
    let mut source = Source::default();
    let failure = dispatch::<Buffered, _, _, _>(&call, &completion, &providers, &mut source)
        .await
        .expect_err("the fixture completion fails");
    assert!(matches!(
        failure,
        BridgeError::Completion(CompletionError::Completion(TestFailure("completion-boom")))
    ));
}

#[tokio::test]
async fn non_streaming_provider_failures_keep_their_source() {
    let params = selector_params(Some("pydantic_ai_agents"), None);
    let message = send_params();
    let call = bridge_call("req-1", &message, &params);
    let completion = recorder();
    let mut providers = provider();
    providers.answer = Err(TestFailure("provider-boom"));
    let mut source = Source::default();
    let failure = dispatch::<Buffered, _, _, _>(&call, &completion, &providers, &mut source)
        .await
        .expect_err("the fixture provider fails");
    assert!(matches!(
        failure,
        BridgeError::Provider(TestFailure("provider-boom"))
    ));
    assert!(recorded_calls(&completion).is_empty());
}

#[rstest]
#[case::provider_not_text(
    litellm_params(&[("custom_llm_provider", parsed("7"))]),
    "custom_llm_provider"
)]
#[case::model_not_text(litellm_params(&[("model", parsed("[]"))]), "model")]
#[tokio::test]
async fn invalid_params_fail_before_any_dispatch(
    #[case] params: BTreeMap<String, AgentJson>,
    #[case] field: &'static str,
) {
    let message = send_params();
    let call = bridge_call("req-1", &message, &params);
    let completion = recorder();
    let providers = provider();
    let mut source = Source::default();
    let failure = dispatch::<Buffered, _, _, _>(&call, &completion, &providers, &mut source)
        .await
        .expect_err("the fixture params are invalid");
    assert!(matches!(
        failure,
        BridgeError::Completion(CompletionError::Params(ParamsError::NotText { field: actual }))
            if actual == field
    ));
    assert!(recorded_calls(&completion).is_empty());
    assert!(selected_configs(&providers).is_empty());
}

#[tokio::test]
async fn streaming_invalid_params_yield_a_single_error() {
    let params = litellm_params(&[("custom_llm_provider", parsed("7"))]);
    let message = send_params();
    let call = bridge_call("req-1", &message, &params);
    let completion = recorder();
    let providers = provider();
    let events = streaming_events(&call, &completion, &providers).await;
    assert_eq!(events.len(), 1);
    assert!(matches!(
        &events[0],
        Err(BridgeError::Completion(CompletionError::Params(
            ParamsError::NotText {
                field: "custom_llm_provider"
            }
        )))
    ));
    assert!(recorded_calls(&completion).is_empty());
}

#[tokio::test]
async fn direct_entries_never_select_a_provider() {
    let params = litellm_params(&[
        (
            "custom_llm_provider",
            AgentJson::String("langflow".to_owned()),
        ),
        ("model", AgentJson::String("flow-1".to_owned())),
    ]);
    let message = send_params();
    let call = bridge_call("req-1", &message, &params);
    let mut completion = recorder();
    completion.answer = Ok(model_response("direct"));
    let mut source = Source::default();
    let result = handle_non_streaming_direct(&call, &completion, &mut source)
        .await
        .expect("the direct completion succeeds");
    assert_eq!(
        result,
        parsed(
            r#"{"jsonrpc":"2.0","id":"req-1","result":{"kind":"message","role":"agent","parts":[{"kind":"text","text":"direct"}],"messageId":"00000000000000000000000000000001"}}"#
        )
    );
    let mut streaming = recorder();
    streaming.chunks = Ok(vec![Ok(chunk("a")), Ok(chunk("b"))]);
    let events = collect_events(handle_streaming_direct(
        &call,
        &streaming,
        Source::default(),
    ))
    .await;
    assert_eq!(
        summaries(&events).join("|"),
        "task:submitted|status:working:false|artifact:ab|status:completed:true"
    );
    let calls = recorded_calls(&streaming);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].model, "langflow/flow-1");
    assert!(calls[0].stream);
}

#[tokio::test]
async fn streaming_accumulates_choice_zero_text() {
    let params = litellm_params(&[("model", AgentJson::String("gpt-4o".to_owned()))]);
    let message = send_params();
    let call = bridge_call("req-1", &message, &params);
    let mut completion = recorder();
    completion.chunks = Ok(vec![
        Ok(chunk("abc")),
        Ok(chunk("")),
        Ok(parsed_chunk(
            r#"{"choices":[{"delta":{"content":"def"}},{"delta":{"content":" ignored"}}]}"#,
        )),
        Ok(parsed_chunk(r#"{"choices":[{"delta":null}]}"#)),
    ]);
    let providers = provider();
    let events = streaming_events(&call, &completion, &providers).await;
    assert_eq!(
        summaries(&events).join("|"),
        "task:submitted|status:working:false|artifact:abcdef|status:completed:true"
    );
    let working = events[1].as_ref().expect("the working event is an event");
    assert_eq!(
        working["result"]["status"]["message"]["parts"][0]["text"],
        "Processing request..."
    );
    assert_eq!(working["result"]["final"], false);
    assert_eq!(
        events[0].as_ref().expect("the task event is an event")["id"],
        "req-1"
    );
    let completed = events[3].as_ref().expect("the completed event is an event");
    assert_eq!(completed["result"]["status"]["state"], "completed");
    assert_eq!(completed["result"]["final"], true);
    let calls = recorded_calls(&completion);
    assert_eq!(calls.len(), 1);
    assert!(calls[0].stream);
    assert_eq!(calls[0].model, "gpt-4o");
}

#[rstest]
#[case::empty_content(vec![chunk("")])]
#[case::missing_delta(vec![parsed_chunk(r#"{"choices":[{}]}"#)])]
#[case::missing_content(vec![parsed_chunk(r#"{"choices":[{"delta":{}}]}"#)])]
#[case::no_choices(vec![parsed_chunk("{}")])]
#[case::empty_choices(vec![parsed_chunk(r#"{"choices":[]}"#)])]
#[case::no_chunks(Vec::new())]
#[tokio::test]
async fn streaming_skips_an_empty_artifact(#[case] chunks: Vec<CompletionChunk>) {
    let params = litellm_params(&[]);
    let message = send_params();
    let call = bridge_call("req-1", &message, &params);
    let mut completion = recorder();
    completion.chunks = Ok(chunks.into_iter().map(Ok).collect());
    let providers = provider();
    let events = streaming_events(&call, &completion, &providers).await;
    assert_eq!(
        summaries(&events).join("|"),
        "task:submitted|status:working:false|status:completed:true"
    );
}

#[tokio::test]
async fn streaming_completion_errors_follow_the_working_status() {
    let params = litellm_params(&[]);
    let message = send_params();
    let call = bridge_call("req-1", &message, &params);
    let providers = provider();
    let mut eager = recorder();
    eager.chunks = Err(TestFailure("stream-boom"));
    let events = streaming_events(&call, &eager, &providers).await;
    assert_eq!(
        summaries(&events[..2]).join("|"),
        "task:submitted|status:working:false"
    );
    assert_eq!(events.len(), 3);
    assert!(matches!(
        &events[2],
        Err(BridgeError::Completion(CompletionError::Completion(
            TestFailure("stream-boom")
        )))
    ));
    let mut streamed = recorder();
    streamed.chunks = Ok(vec![Ok(chunk("abc")), Err(TestFailure("chunk-boom"))]);
    let streamed_events = streaming_events(&call, &streamed, &providers).await;
    assert_eq!(
        summaries(&streamed_events[..2]).join("|"),
        "task:submitted|status:working:false"
    );
    assert_eq!(streamed_events.len(), 3);
    assert!(matches!(
        &streamed_events[2],
        Err(BridgeError::Completion(CompletionError::Completion(
            TestFailure("chunk-boom")
        )))
    ));
}

#[tokio::test]
async fn streaming_routes_to_the_selected_config() {
    let params = selector_params(Some("pydantic_ai_agents"), None);
    let message = send_params();
    let call = bridge_call("req-1", &message, &params);
    let completion = recorder();
    let mut providers = provider();
    providers.events = Ok(vec![
        Ok(parsed(
            r#"{"result":{"kind":"task","status":{"state":"submitted"}}}"#,
        )),
        Ok(parsed(
            r#"{"result":{"kind":"status-update","status":{"state":"working"},"final":false}}"#,
        )),
    ]);
    let events = streaming_events(&call, &completion, &providers).await;
    assert_eq!(
        summaries(&events).join("|"),
        "task:submitted|status:working:false"
    );
    assert!(recorded_calls(&completion).is_empty());
    assert_eq!(
        selected_configs(&providers),
        vec![A2AProviderConfig::PydanticAI]
    );
}

#[tokio::test]
async fn streaming_provider_failures_are_a_single_error() {
    let params = selector_params(Some("langflow"), None);
    let message = send_params();
    let call = bridge_call("req-1", &message, &params);
    let completion = recorder();
    let mut providers = provider();
    providers.events = Err(TestFailure("provider-boom"));
    let events = streaming_events(&call, &completion, &providers).await;
    assert_eq!(events.len(), 1);
    assert!(matches!(
        &events[0],
        Err(BridgeError::Provider(TestFailure("provider-boom")))
    ));
}

#[tokio::test]
async fn provider_configs_receive_the_original_call() {
    let mut params = selector_params(Some("pydantic_ai_agents"), None);
    params.insert(A2A_USER_API_KEY_HASH_PARAM.to_owned(), parsed(r#""hash""#));
    let message = send_params();
    let mut call = bridge_call("req-1", &message, &params);
    call.api_base = Some("https://card.example/v1");
    let dynamic = BTreeMap::from([("X-Trace".to_owned(), "dynamic".to_owned())]);
    call.agent_extra_headers = Some(&dynamic);
    let completion = recorder();
    let providers = provider();
    let mut source = Source::default();
    dispatch::<Buffered, _, _, _>(&call, &completion, &providers, &mut source)
        .await
        .expect("the fixture dispatch succeeds");
    let events = streaming_events(&call, &completion, &providers).await;
    assert!(events.is_empty());
    let received = provider_calls(&providers);
    assert_eq!(received.len(), 2);
    assert_eq!(received[0].litellm_params, params);
    assert_eq!(received[1].litellm_params, params);
    assert_eq!(
        received[0].api_base.as_deref(),
        Some("https://card.example/v1")
    );
    assert_eq!(
        received[1].api_base.as_deref(),
        Some("https://card.example/v1")
    );
    assert_eq!(received[0].agent_extra_headers.as_ref(), Some(&dynamic));
    assert_eq!(received[1].agent_extra_headers.as_ref(), Some(&dynamic));
}

#[tokio::test]
async fn streaming_stops_after_the_text_cap() {
    let params = litellm_params(&[]);
    let message = send_params();
    let call = bridge_call("req-1", &message, &params);
    let mut completion = recorder();
    let full = "x".repeat(MAX_TEXT_BYTES);
    completion.chunks = Ok(vec![Ok(chunk(&full)), Ok(chunk("y"))]);
    let providers = provider();
    let events = streaming_events(&call, &completion, &providers).await;
    assert_eq!(
        summaries(&events[..2]).join("|"),
        "task:submitted|status:working:false"
    );
    assert_eq!(events.len(), 3);
    assert!(matches!(
        &events[2],
        Err(BridgeError::Completion(CompletionError::TextTooLong {
            max: MAX_TEXT_BYTES
        }))
    ));
}

#[tokio::test]
async fn streaming_accepts_text_exactly_at_the_cap() {
    let params = litellm_params(&[]);
    let message = send_params();
    let call = bridge_call("req-1", &message, &params);
    let mut completion = recorder();
    let full = "x".repeat(MAX_TEXT_BYTES);
    completion.chunks = Ok(vec![Ok(chunk(&full))]);
    let providers = provider();
    let events = streaming_events(&call, &completion, &providers).await;
    assert_eq!(events.len(), 4);
    let artifact = events[2].as_ref().expect("the artifact is an event");
    assert_eq!(artifact["result"]["kind"], "artifact-update");
    assert_eq!(
        artifact["result"]["artifact"]["parts"][0]["text"]
            .as_str()
            .map(str::len),
        Some(MAX_TEXT_BYTES)
    );
}

async fn streaming_events<C, P>(
    call: &BridgeCall<'_>,
    completion: &C,
    providers: &P,
) -> Vec<Result<AgentJson, BridgeError<C::Error, P::Error>>>
where
    C: CompletionPort,
    P: A2AProviderDispatch,
{
    let mut source = Source::default();
    collect_events(dispatch::<Streamed, _, _, _>(call, completion, providers, &mut source).await)
        .await
}

fn parsed_chunk(text: &str) -> CompletionChunk {
    serde_json::from_str(text).expect("the fixture chunk is a completion chunk")
}

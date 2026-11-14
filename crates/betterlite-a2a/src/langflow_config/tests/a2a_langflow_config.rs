use std::collections::BTreeMap;

use betterlite_core::agents::json::AgentJson;
use rstest::rstest;

use crate::completion_bridge::dispatch::CompletionError;
use crate::completion_bridge::params::ParamsError;
use crate::completion_bridge::tests::Source;
use crate::completion_bridge::tests::support::{
    TestFailure, bridge_call, chunk, collect_events, litellm_params, parsed, recorded_calls,
    recorder, send_params, send_params_with_context, summaries,
};
use crate::langflow_config::{
    LangFlowConfigError, LangFlowError, handle_non_streaming, handle_streaming,
};
use crate::utils::session_scoping::SessionScopeError;

const MISSING_PARAMS: &str =
    "litellm_params is required for LangFlowA2AConfig (must contain custom_llm_provider and model)";

fn langflow_params(principal: Option<AgentJson>) -> BTreeMap<String, AgentJson> {
    let mut params = litellm_params(&[
        (
            "custom_llm_provider",
            AgentJson::String("langflow".to_owned()),
        ),
        ("model", AgentJson::String("flow-1".to_owned())),
    ]);
    if let Some(hash) = principal {
        params.insert("litellm_a2a_user_api_key_hash".to_owned(), hash);
    }
    params
}

#[tokio::test]
async fn scopes_the_session_to_the_principal() {
    let params = langflow_params(Some(parsed(r#""secret""#)));
    let message = send_params_with_context("conv-1");
    let mut call = bridge_call("req-1", &message, &params);
    call.api_base = Some("https://card.example/v1");
    let completion = recorder();
    let mut source = Source::default();
    let result = handle_non_streaming(&call, &completion, &mut source)
        .await
        .expect("the fixture completion succeeds");
    assert_eq!(result["id"], "req-1");
    let calls = recorded_calls(&completion);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].model, "langflow/flow-1");
    assert!(!calls[0].stream);
    assert_eq!(
        calls[0].extra.get("session_id"),
        Some(&parsed(r#""2bb80d537b1da3e3-conv-1""#))
    );
    assert_eq!(
        calls[0].extra.get("api_base"),
        Some(&parsed(r#""https://card.example/v1""#))
    );
    assert!(!calls[0].extra.contains_key("litellm_a2a_user_api_key_hash"));
}

#[rstest]
#[case::no_principal(None, "conv-1")]
#[case::null_principal(Some(AgentJson::Null), "conv-1")]
#[case::empty_principal(Some(parsed(r#""""#)), "conv-1")]
#[tokio::test]
async fn keeps_the_raw_session_without_a_principal(
    #[case] principal: Option<AgentJson>,
    #[case] expected: &str,
) {
    let params = langflow_params(principal);
    let message = send_params_with_context("conv-1");
    let call = bridge_call("req-1", &message, &params);
    let completion = recorder();
    let mut source = Source::default();
    handle_non_streaming(&call, &completion, &mut source)
        .await
        .expect("the fixture completion succeeds");
    let calls = recorded_calls(&completion);
    assert_eq!(
        calls[0].extra.get("session_id"),
        Some(&AgentJson::String(expected.to_owned()))
    );
}

#[rstest]
#[case::configured(parsed(r#""server-session""#))]
#[case::null(AgentJson::Null)]
#[case::empty(parsed(r#""""#))]
#[tokio::test]
async fn configured_sessions_are_not_overwritten(#[case] session: AgentJson) {
    let mut params = langflow_params(Some(parsed(r#""secret""#)));
    params.insert("session_id".to_owned(), session.clone());
    let message = send_params_with_context("conv-1");
    let call = bridge_call("req-1", &message, &params);
    let completion = recorder();
    let mut source = Source::default();
    handle_non_streaming(&call, &completion, &mut source)
        .await
        .expect("the fixture completion succeeds");
    let calls = recorded_calls(&completion);
    assert_eq!(calls[0].extra.get("session_id"), Some(&session));
}

#[rstest]
#[case::absent(None)]
#[case::null(Some(AgentJson::Null))]
#[case::empty(Some(parsed(r#""""#)))]
#[tokio::test]
async fn absent_or_empty_context_ids_add_no_session(#[case] context_id: Option<AgentJson>) {
    let params = langflow_params(Some(parsed(r#""secret""#)));
    let mut message = send_params();
    if let Some(value) = context_id {
        message.message.extra.insert("contextId".to_owned(), value);
    }
    let call = bridge_call("req-1", &message, &params);
    let completion = recorder();
    let mut source = Source::default();
    handle_non_streaming(&call, &completion, &mut source)
        .await
        .expect("the fixture completion succeeds");
    let calls = recorded_calls(&completion);
    assert!(!calls[0].extra.contains_key("session_id"));
}

#[tokio::test]
async fn non_string_context_ids_are_rejected() {
    let params = langflow_params(Some(parsed(r#""secret""#)));
    let mut message = send_params();
    message
        .message
        .extra
        .insert("contextId".to_owned(), parsed("7"));
    let call = bridge_call("req-1", &message, &params);
    let completion = recorder();
    let mut source = Source::default();
    let failure = handle_non_streaming(&call, &completion, &mut source)
        .await
        .expect_err("the fixture contextId is invalid");
    assert!(matches!(
        failure,
        LangFlowError::Config(LangFlowConfigError::Session(
            SessionScopeError::ContextIdType
        ))
    ));
    assert!(recorded_calls(&completion).is_empty());
}

#[tokio::test]
async fn non_string_principals_are_rejected() {
    let params = langflow_params(Some(parsed("7")));
    let message = send_params_with_context("conv-1");
    let call = bridge_call("req-1", &message, &params);
    let completion = recorder();
    let mut source = Source::default();
    let failure = handle_non_streaming(&call, &completion, &mut source)
        .await
        .expect_err("the fixture principal is invalid");
    assert!(matches!(
        failure,
        LangFlowError::Config(LangFlowConfigError::Params(ParamsError::NotText {
            field: "litellm_a2a_user_api_key_hash"
        }))
    ));
}

#[tokio::test]
async fn empty_litellm_params_are_rejected() {
    let params = litellm_params(&[]);
    let message = send_params();
    let call = bridge_call("req-1", &message, &params);
    let completion = recorder();
    let mut source = Source::default();
    let failure = handle_non_streaming(&call, &completion, &mut source)
        .await
        .expect_err("the fixture litellm_params are empty");
    assert_eq!(failure.to_string(), MISSING_PARAMS);
    assert!(matches!(
        &failure,
        LangFlowError::Config(LangFlowConfigError::MissingLitellmParams)
    ));
    let events = collect_events(handle_streaming(&call, &completion, Source::default())).await;
    assert_eq!(events.len(), 1);
    assert!(matches!(
        &events[0],
        Err(LangFlowError::Config(
            LangFlowConfigError::MissingLitellmParams
        ))
    ));
    assert!(recorded_calls(&completion).is_empty());
}

#[tokio::test]
async fn streaming_scopes_the_session_before_dispatch() {
    let params = langflow_params(Some(parsed(r#""secret""#)));
    let message = send_params_with_context("conv-1");
    let call = bridge_call("req-1", &message, &params);
    let mut completion = recorder();
    completion.chunks = Ok(vec![Ok(chunk("Hi"))]);
    let events = collect_events(handle_streaming(&call, &completion, Source::default())).await;
    assert_eq!(
        summaries(&events).join("|"),
        "task:submitted|status:working:false|artifact:Hi|status:completed:true"
    );
    let calls = recorded_calls(&completion);
    assert!(calls[0].stream);
    assert_eq!(
        calls[0].extra.get("session_id"),
        Some(&parsed(r#""2bb80d537b1da3e3-conv-1""#))
    );
}

#[tokio::test]
async fn agent_headers_are_not_forwarded() {
    let params = langflow_params(None);
    let message = send_params();
    let mut call = bridge_call("req-1", &message, &params);
    let dynamic = BTreeMap::from([("X-Trace".to_owned(), "dynamic".to_owned())]);
    call.agent_extra_headers = Some(&dynamic);
    let completion = recorder();
    let mut source = Source::default();
    handle_non_streaming(&call, &completion, &mut source)
        .await
        .expect("the fixture completion succeeds");
    let calls = recorded_calls(&completion);
    assert!(!calls[0].extra.contains_key("extra_headers"));
}

#[tokio::test]
async fn streaming_agent_headers_are_not_forwarded() {
    let params = langflow_params(None);
    let message = send_params();
    let mut call = bridge_call("req-1", &message, &params);
    let dynamic = BTreeMap::from([("X-Trace".to_owned(), "dynamic".to_owned())]);
    call.agent_extra_headers = Some(&dynamic);
    let completion = recorder();
    let events = collect_events(handle_streaming(&call, &completion, Source::default())).await;
    assert_eq!(
        summaries(&events).join("|"),
        "task:submitted|status:working:false|status:completed:true"
    );
    let calls = recorded_calls(&completion);
    assert!(!calls[0].extra.contains_key("extra_headers"));
}

#[tokio::test]
async fn streaming_completion_errors_stay_inside_the_stream() {
    let params = langflow_params(Some(parsed(r#""secret""#)));
    let message = send_params_with_context("conv-1");
    let call = bridge_call("req-1", &message, &params);
    let mut completion = recorder();
    completion.chunks = Err(TestFailure("stream-boom"));
    let events = collect_events(handle_streaming(&call, &completion, Source::default())).await;
    assert_eq!(
        summaries(&events[..2]).join("|"),
        "task:submitted|status:working:false"
    );
    assert_eq!(events.len(), 3);
    assert!(matches!(
        &events[2],
        Err(LangFlowError::Completion(CompletionError::Completion(
            TestFailure("stream-boom")
        )))
    ));
}

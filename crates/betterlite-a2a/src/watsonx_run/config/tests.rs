#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_core::agents::json::AgentJson;

use super::{Answer, ConfigCall, ConfigError, Reply, WatsonxRunConfig};
use crate::watsonx_credentials::token_cache::BoundedTokenCache;
use crate::watsonx_run::handler::RunError;
use crate::watsonx_run::tests::{
    Clock, Reply as ScriptReply, Script, ScriptError, Source, kwargs, run_handler, send_params,
    token_reply,
};
use crate::watsonx_transform::extract::WxoSendParams;
use crate::watsonx_transform::fake_streaming::WxoStreamResult;

fn call<'call>(
    params: &'call WxoSendParams,
    kwargs: &'call BTreeMap<String, AgentJson>,
) -> ConfigCall<'call> {
    ConfigCall {
        request_id: "req-1",
        params,
        kwargs,
    }
}

fn config(script: &Script) -> WatsonxRunConfig<&Script, &Script, Clock, BoundedTokenCache, Source> {
    WatsonxRunConfig {
        handler: run_handler(script),
    }
}

#[tokio::test]
async fn config_extracts_litellm_params_and_returns_the_a2a_message() {
    let script = Script::new(vec![
        token_reply(),
        ScriptReply::json(
            r#"{"run_id": "run-1", "status": "completed", "last_message": {"content": [{"text": "hi"}]}}"#,
        ),
    ]);
    let params = send_params();
    let kwargs = kwargs();
    let Answer::Message(response) = config(&script)
        .handle(&call(&params, &kwargs), Reply::Message)
        .await
        .unwrap()
    else {
        panic!("expected a message answer");
    };
    assert_eq!(
        serde_json::to_string(&response).unwrap(),
        concat!(
            r#"{"id":"req-1","jsonrpc":"2.0","result":{"kind":"message","role":"agent","#,
            r#""parts":[{"kind":"text","text":"hi"}],"#,
            r#""messageId":"00000000-0000-0000-0000-000000000001"}}"#,
        )
    );
}

#[tokio::test]
async fn config_streams_the_fake_events() {
    let script = Script::new(vec![
        token_reply(),
        ScriptReply::sse(&[
            "data: {\"last_message\": {\"content\": [{\"text\": \"hi\"}]}}",
            "data: [DONE]",
        ]),
    ]);
    let params = send_params();
    let kwargs = kwargs();
    let Answer::Stream(steps) = config(&script)
        .handle(&call(&params, &kwargs), Reply::Stream)
        .await
        .unwrap()
    else {
        panic!("expected a stream answer");
    };
    assert_eq!(steps.len(), 4);
    let WxoStreamResult::Artifact(artifact) = &steps[2].event.result else {
        panic!("expected an artifact");
    };
    assert_eq!(artifact.artifact.parts[0].text.as_deref(), Some("hi"));
}

#[tokio::test]
async fn config_fails_when_litellm_params_is_missing() {
    let script = Script::new(Vec::new());
    let params = send_params();
    let kwargs = BTreeMap::new();
    let error: ConfigError<ScriptError, ScriptError> = config(&script)
        .handle(&call(&params, &kwargs), Reply::Message)
        .await
        .unwrap_err();
    assert!(matches!(error, ConfigError::MissingLitellmParams));
}

#[tokio::test]
async fn config_fails_when_litellm_params_is_not_a_mapping() {
    let script = Script::new(Vec::new());
    let params = send_params();
    let kwargs = BTreeMap::from([(
        "litellm_params".to_owned(),
        serde_json::from_str::<AgentJson>("5").unwrap(),
    )]);
    let error = config(&script)
        .handle(&call(&params, &kwargs), Reply::Stream)
        .await
        .unwrap_err();
    assert!(matches!(error, ConfigError::LitellmParams(_)));
}

#[tokio::test]
async fn config_fails_when_a_required_param_is_missing() {
    let script = Script::new(Vec::new());
    let params = send_params();
    let kwargs = BTreeMap::from([(
        "litellm_params".to_owned(),
        serde_json::from_str::<AgentJson>("{}").unwrap(),
    )]);
    let error = config(&script)
        .handle(&call(&params, &kwargs), Reply::Message)
        .await
        .unwrap_err();
    assert!(matches!(error, ConfigError::Run(RunError::Params(_))));
}

#[tokio::test]
async fn config_ignores_the_other_kwargs() {
    let script = Script::new(vec![
        token_reply(),
        ScriptReply::json(
            r#"{"run_id": "run-1", "status": "completed", "last_message": {"content": [{"text": "hi"}]}}"#,
        ),
    ]);
    let params = send_params();
    let mut kwargs = kwargs();
    kwargs.insert("trace_id".to_owned(), AgentJson::String("t-1".to_owned()));
    config(&script)
        .handle(&call(&params, &kwargs), Reply::Message)
        .await
        .unwrap();
    assert_eq!(script.recorded().len(), 1);
}

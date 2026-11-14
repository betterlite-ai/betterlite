use std::collections::BTreeMap;
use std::fmt::Debug;
use std::future::{Future, ready};
use std::sync::Mutex;

use betterlite_core::agents::json::AgentJson;
use futures_util::stream::{self, Stream, StreamExt as _};
use thiserror::Error;

use crate::completion_bridge::completion::{
    CompletionCall, CompletionChunk, CompletionPort, StreamedChunks,
};
use crate::completion_bridge::dispatch::{A2AProviderDispatch, BridgeEvents};
use crate::completion_bridge::params::BridgeCall;
use crate::completion_bridge::wire::{A2AMessage, MessageSendParams, ModelResponse, Role};
use crate::provider_selector::A2AProviderConfig;

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("{0}")]
pub(crate) struct TestFailure(pub(crate) &'static str);

pub(crate) struct RecordingCompletion {
    pub(crate) answer: Result<ModelResponse, TestFailure>,
    pub(crate) chunks: Result<Vec<Result<CompletionChunk, TestFailure>>, TestFailure>,
    pub(crate) calls: Mutex<Vec<CompletionCall>>,
}

pub(crate) fn recorder() -> RecordingCompletion {
    RecordingCompletion {
        answer: Ok(ModelResponse::default()),
        chunks: Ok(Vec::new()),
        calls: Mutex::new(Vec::new()),
    }
}

impl RecordingCompletion {
    fn record(&self, call: CompletionCall) {
        self.calls
            .lock()
            .expect("the fixture lock is not poisoned")
            .push(call);
    }
}

impl CompletionPort for RecordingCompletion {
    type Error = TestFailure;

    fn complete(
        &self,
        call: CompletionCall,
    ) -> impl Future<Output = Result<ModelResponse, Self::Error>> + Send {
        self.record(call);
        ready(self.answer.clone())
    }

    fn stream(
        &self,
        call: CompletionCall,
    ) -> impl Future<Output = Result<StreamedChunks<Self::Error>, Self::Error>> + Send {
        self.record(call);
        ready(
            self.chunks
                .clone()
                .map(|chunks| stream::iter(chunks).boxed()),
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProviderCall {
    pub(crate) litellm_params: BTreeMap<String, AgentJson>,
    pub(crate) api_base: Option<String>,
    pub(crate) agent_extra_headers: Option<BTreeMap<String, String>>,
}

pub(crate) struct RecordingProvider {
    pub(crate) answer: Result<AgentJson, TestFailure>,
    pub(crate) events: Result<Vec<Result<AgentJson, TestFailure>>, TestFailure>,
    pub(crate) configs: Mutex<Vec<A2AProviderConfig>>,
    pub(crate) calls: Mutex<Vec<ProviderCall>>,
}

pub(crate) fn provider() -> RecordingProvider {
    RecordingProvider {
        answer: Ok(parsed(
            r#"{"jsonrpc":"2.0","id":"provider","result":{"kind":"message","role":"agent","parts":[{"kind":"text","text":"from the provider"}],"messageId":"provider-message"}}"#,
        )),
        events: Ok(Vec::new()),
        configs: Mutex::new(Vec::new()),
        calls: Mutex::new(Vec::new()),
    }
}

fn provider_call(call: &BridgeCall<'_>) -> ProviderCall {
    ProviderCall {
        litellm_params: call.litellm_params.clone(),
        api_base: call.api_base.map(str::to_owned),
        agent_extra_headers: call.agent_extra_headers.cloned(),
    }
}

impl RecordingProvider {
    fn record(&self, config: A2AProviderConfig, call: &BridgeCall<'_>) {
        self.configs
            .lock()
            .expect("the fixture lock is not poisoned")
            .push(config);
        self.calls
            .lock()
            .expect("the fixture lock is not poisoned")
            .push(provider_call(call));
    }

    fn recorded_events(
        &self,
        config: A2AProviderConfig,
        call: &BridgeCall<'_>,
    ) -> Result<BridgeEvents<'static, TestFailure>, TestFailure> {
        self.record(config, call);
        self.events
            .clone()
            .map(|events| stream::iter(events).boxed())
    }
}

impl A2AProviderDispatch for RecordingProvider {
    type Error = TestFailure;

    fn handle_non_streaming(
        &self,
        config: A2AProviderConfig,
        call: &BridgeCall<'_>,
    ) -> impl Future<Output = Result<AgentJson, Self::Error>> + Send {
        self.record(config, call);
        ready(self.answer.clone())
    }

    fn handle_streaming(
        &self,
        config: A2AProviderConfig,
        call: &BridgeCall<'_>,
    ) -> impl Future<Output = Result<BridgeEvents<'static, Self::Error>, Self::Error>> + Send {
        ready(self.recorded_events(config, call))
    }
}

pub(crate) fn parsed(text: &str) -> AgentJson {
    serde_json::from_str(text).expect("the fixture text is JSON")
}

pub(crate) fn chunk(text: &str) -> CompletionChunk {
    serde_json::from_value(parsed(&format!(
        r#"{{"choices":[{{"delta":{{"content":{}}}}}]}}"#,
        serde_json::to_string(text).expect("the fixture text is a string")
    )))
    .expect("the fixture chunk is a completion chunk")
}

pub(crate) fn model_response(text: &str) -> ModelResponse {
    serde_json::from_value(parsed(&format!(
        r#"{{"choices":[{{"message":{{"content":{}}}}}]}}"#,
        serde_json::to_string(text).expect("the fixture text is a string")
    )))
    .expect("the fixture response is a model response")
}

pub(crate) fn litellm_params(entries: &[(&str, AgentJson)]) -> BTreeMap<String, AgentJson> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

pub(crate) fn selector_params(
    provider: Option<&str>,
    model: Option<&str>,
) -> BTreeMap<String, AgentJson> {
    let mut params = litellm_params(&[]);
    if let Some(name) = provider {
        params.insert(
            "custom_llm_provider".to_owned(),
            AgentJson::String(name.to_owned()),
        );
    }
    if let Some(name) = model {
        params.insert("model".to_owned(), AgentJson::String(name.to_owned()));
    }
    params
}

pub(crate) fn recorded_calls(completion: &RecordingCompletion) -> Vec<CompletionCall> {
    completion
        .calls
        .lock()
        .expect("the fixture lock is not poisoned")
        .clone()
}

pub(crate) fn selected_configs(providers: &RecordingProvider) -> Vec<A2AProviderConfig> {
    providers
        .configs
        .lock()
        .expect("the fixture lock is not poisoned")
        .clone()
}

pub(crate) fn provider_calls(providers: &RecordingProvider) -> Vec<ProviderCall> {
    providers
        .calls
        .lock()
        .expect("the fixture lock is not poisoned")
        .clone()
}

pub(crate) fn send_params() -> MessageSendParams {
    message_params(None)
}

pub(crate) fn send_params_with_context(context_id: &str) -> MessageSendParams {
    message_params(Some(context_id))
}

fn message_params(context_id: Option<&str>) -> MessageSendParams {
    let mut params = MessageSendParams {
        message: A2AMessage {
            role: Role("user".to_owned()),
            parts: vec![parsed(r#"{"kind":"text","text":"hello"}"#)],
            message_id: Some("message-1".to_owned()),
            metadata: None,
            extra: BTreeMap::new(),
        },
        metadata: None,
        extra: BTreeMap::new(),
    };
    if let Some(context_id) = context_id {
        params.message.extra.insert(
            "contextId".to_owned(),
            AgentJson::String(context_id.to_owned()),
        );
    }
    params
}

pub(crate) fn send_params_with_metadata(
    message_metadata: Option<AgentJson>,
    request_metadata: Option<AgentJson>,
) -> MessageSendParams {
    let mut params = send_params();
    params.message.metadata = message_metadata;
    params.metadata = request_metadata;
    params
}

pub(crate) fn bridge_call<'call>(
    request_id: &'call str,
    params: &'call MessageSendParams,
    litellm_params: &'call BTreeMap<String, AgentJson>,
) -> BridgeCall<'call> {
    BridgeCall {
        request_id,
        params,
        litellm_params,
        api_base: None,
        agent_extra_headers: None,
    }
}

pub(crate) async fn collect_events<S, E>(events: S) -> Vec<Result<AgentJson, E>>
where
    S: Stream<Item = Result<AgentJson, E>>,
{
    let mut pinned = Box::pin(events);
    let mut collected = Vec::new();
    while let Some(next) = pinned.next().await {
        collected.push(next);
    }
    collected
}

pub(crate) fn summaries<E>(events: &[Result<AgentJson, E>]) -> Vec<String>
where
    E: Debug,
{
    events
        .iter()
        .map(|event| summarize(event.as_ref().expect("the fixture event stream succeeds")))
        .collect()
}

pub(crate) fn summarize(event: &AgentJson) -> String {
    let result = &event["result"];
    match result["kind"]
        .as_str()
        .expect("the fixture event carries a result kind")
    {
        "task" => format!(
            "task:{}",
            result["status"]["state"]
                .as_str()
                .expect("the task carries a state")
        ),
        "status-update" => format!(
            "status:{}:{}",
            result["status"]["state"]
                .as_str()
                .expect("the status carries a state"),
            result["final"]
                .as_bool()
                .expect("the status carries a final flag")
        ),
        "artifact-update" => format!(
            "artifact:{}",
            result["artifact"]["parts"][0]["text"]
                .as_str()
                .expect("the artifact carries its text")
        ),
        other => panic!("unexpected event kind {other}"),
    }
}

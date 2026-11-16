use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use betterlite_models::decisions::Json;
use betterlite_streaming::stream_chunk_shaping::records::JsonValue;
use betterlite_streaming::stream_usage_assembly::wire::Usage;
use betterlite_streaming::streamed_response_assembly::AssemblyError;
use betterlite_streaming::streamed_response_assembly::response::{
    ChatChoice, ChatMessage, ChatResponse, Response, TextResponse,
};
use indexmap::IndexMap;

use crate::api::api_boundaries::ApiLimits;
use crate::api::chat_response_stream_tools::state::{ChatStreamState, StreamSetup};

use super::super::error::StreamItemsError;
use super::super::snapshot::StreamAssembly;
use super::super::sources::EventSources;

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the fixture is JSON")
}

pub(super) fn state_with_details(
    request: &str,
    provider: Option<&str>,
    litellm_metadata: Option<IndexMap<String, Json>>,
) -> ChatStreamState<String> {
    let request = json(request);
    ChatStreamState::new(StreamSetup {
        model: "gpt-4o-mini",
        logging_context: String::from("wrapper-context"),
        request_input: &Json::Text(String::from("hi")),
        responses_api_request: &request,
        custom_llm_provider: provider,
        litellm_metadata,
        limits: ApiLimits::default(),
    })
    .expect("the state builds")
}

pub(super) fn sources(now: i64, ids: &[&str]) -> EventSources {
    let queue: Arc<Mutex<VecDeque<String>>> =
        Arc::new(Mutex::new(ids.iter().map(|id| (*id).to_owned()).collect()));
    let next = Arc::clone(&queue);
    EventSources {
        clock: Arc::new(move || Ok::<i64, StreamItemsError>(now)),
        ids: Arc::new(move || {
            next.lock()
                .expect("the id queue locks")
                .pop_front()
                .unwrap_or_default()
        }),
    }
}

pub(super) fn field(value: &Json, name: &str) -> Json {
    match value {
        Json::Object(fields) => fields.get(name).cloned().unwrap_or(Json::Null),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => Json::Null,
    }
}

pub(super) fn path(value: &Json, names: &[&str]) -> Json {
    let mut current = value.clone();
    for name in names {
        current = field(&current, name);
    }
    current
}

pub(super) fn completion(content: &Json, annotations: &str) -> Json {
    let content = serde_json::to_string(content).expect("the content serializes");
    json(&format!(
        r#"{{"choices":[{{"message":{{"content":{content},"annotations":{annotations}}}}}]}}"#
    ))
}

#[derive(Clone, Copy)]
pub(super) enum AssemblyResponse {
    None,
    Chat,
    Text,
}

pub(super) struct FixtureAssembly {
    pub(super) response: AssemblyResponse,
    pub(super) received: Arc<Mutex<Vec<Vec<IndexMap<String, Json>>>>>,
}

impl FixtureAssembly {
    pub(super) fn new(response: AssemblyResponse) -> Self {
        Self {
            response,
            received: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl StreamAssembly for FixtureAssembly {
    fn assemble(
        &self,
        chunks: &[IndexMap<String, Json>],
    ) -> Result<Option<Response>, AssemblyError> {
        self.received
            .lock()
            .expect("the calls lock")
            .push(chunks.to_vec());
        match self.response {
            AssemblyResponse::None => Ok(None),
            AssemblyResponse::Chat => Ok(Some(chat_response("stored", BTreeMap::new()))),
            AssemblyResponse::Text => Ok(Some(Response::Text(TextResponse::default()))),
        }
    }
}

pub(super) fn chat_response(content: &str, hidden: BTreeMap<String, JsonValue>) -> Response {
    let message = ChatMessage {
        content: Some(content.to_owned()),
        role: String::from("assistant"),
        tool_calls: None,
        function_call: None,
        provider_specific_fields: None,
        reasoning_content: None,
        thinking_blocks: None,
        annotations: None,
        audio: None,
        images: None,
    };
    let choice = ChatChoice {
        index: 0,
        message,
        finish_reason: String::from("stop"),
        provider_specific_fields: None,
    };
    Response::Chat(Box::new(ChatResponse {
        id: String::from("chatcmpl-1"),
        object: String::from("chat.completion"),
        created: 1,
        model: String::from("gpt-4o-mini"),
        system_fingerprint: None,
        service_tier: None,
        choices: vec![choice],
        usage: Usage::default(),
        hidden_params: hidden,
    }))
}

pub(super) fn state_with_assembly(
    request: &str,
    assembly: FixtureAssembly,
) -> ChatStreamState<FixtureAssembly> {
    let request = json(request);
    ChatStreamState::new(StreamSetup {
        model: "gpt-4o-mini",
        logging_context: assembly,
        request_input: &Json::Text(String::from("hi")),
        responses_api_request: &request,
        custom_llm_provider: None,
        litellm_metadata: None,
        limits: ApiLimits::default(),
    })
    .expect("the state builds")
}

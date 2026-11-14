use std::collections::VecDeque;
use std::convert::Infallible;
use std::fmt::Debug;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_openai::chat::response::ChatResponse;
use indexmap::IndexMap;
use serde_json::Value;
use thiserror::Error;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use crate::api::skills::continuation::error::SkillsLoopError;
use crate::api::skills::continuation::messages::{ContinuationRequest, MessagesContinuation};
use crate::api::skills::continuation::request::{RequestMetadata, SkillsRequestData};
use crate::api::skills::hook::SkillsInjectionHook;
use crate::api::skills::hook::execution::{SandboxFile, SandboxOutcome, SkillFiles, SkillsSandbox};
use crate::api::skills::hook::response::{
    ContentBlock, GeneratedFilesAttribute, HookResponse, ObjectResponse,
};
use crate::api::skills::hook::settings::{DEFAULT_SANDBOX_TIMEOUT, HookSettings};

#[derive(Debug, Error)]
#[error("the scripted continuation failed")]
pub(super) struct ScriptedFailure;

pub(super) type Reply = Result<Option<HookResponse>, ScriptedFailure>;

pub(super) struct ScriptedSandbox {
    outcomes: Mutex<VecDeque<SandboxOutcome>>,
    pub(super) calls: Mutex<Vec<(String, SkillFiles)>>,
}

impl SkillsSandbox for ScriptedSandbox {
    type Error = Infallible;

    fn execute(
        &self,
        code: &str,
        skill_files: &SkillFiles,
    ) -> impl Future<Output = Result<SandboxOutcome, Self::Error>> + Send {
        self.calls
            .lock()
            .unwrap()
            .push((code.to_owned(), skill_files.clone()));
        let outcome = self
            .outcomes
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_default();
        ready(Ok(outcome))
    }
}

impl ScriptedSandbox {
    pub(super) fn codes(&self) -> Vec<String> {
        let calls = self.calls.lock().unwrap();
        calls.iter().map(|(code, _)| code.clone()).collect()
    }
}

pub(super) struct ScriptedContinuation {
    replies: Mutex<VecDeque<Reply>>,
    requests: Mutex<Vec<ContinuationRequest>>,
}

impl ScriptedContinuation {
    pub(super) fn new(replies: Vec<Reply>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from(replies)),
            requests: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn requests(&self) -> Vec<ContinuationRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl MessagesContinuation for ScriptedContinuation {
    type Error = ScriptedFailure;

    fn create(&self, request: &ContinuationRequest) -> impl Future<Output = Reply> + Send {
        self.requests.lock().unwrap().push(request.clone());
        let reply = self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("the continuation was called more often than the test scripted");
        ready(reply)
    }
}

pub(super) fn hook(
    max_iterations: u32,
    outcomes: Vec<SandboxOutcome>,
) -> SkillsInjectionHook<(), ScriptedSandbox> {
    SkillsInjectionHook {
        prompt_handler: (),
        executor: ScriptedSandbox {
            outcomes: Mutex::new(VecDeque::from(outcomes)),
            calls: Mutex::new(Vec::new()),
        },
        settings: HookSettings {
            max_iterations,
            sandbox_timeout: DEFAULT_SANDBOX_TIMEOUT,
        },
    }
}

pub(super) fn json(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

pub(super) fn outcome(output: &str) -> SandboxOutcome {
    SandboxOutcome {
        output: Some(output.to_owned()),
        error: None,
        files: Vec::new(),
    }
}

pub(super) fn outcome_with_file(output: &str, name: &str, content_base64: &str) -> SandboxOutcome {
    SandboxOutcome {
        output: Some(output.to_owned()),
        error: None,
        files: vec![SandboxFile {
            name: name.to_owned(),
            mime_type: "image/png".to_owned(),
            content_base64: content_base64.to_owned(),
        }],
    }
}

pub(super) fn skill_files(entries: &[(&str, &str)]) -> SkillFiles {
    entries
        .iter()
        .map(|(path, content)| ((*path).to_owned(), content.as_bytes().to_vec()))
        .collect()
}

pub(super) fn request_with(skill_files: IndexMap<String, SkillFiles>) -> SkillsRequestData {
    SkillsRequestData {
        model: "claude-sonnet".to_owned(),
        messages: vec![json(r#"{"role":"user","content":"make a chart"}"#)],
        tools: vec![json(r#"{"name":"litellm_code_execution"}"#)],
        max_tokens: None,
        skills: RequestMetadata {
            code_execution_enabled: true,
            skill_files,
        },
    }
}

pub(super) fn request() -> SkillsRequestData {
    let mut by_skill = IndexMap::new();
    by_skill.insert(
        "litellm_skill_one".to_owned(),
        skill_files(&[("core/gif_builder.py", "class GifBuilder: pass")]),
    );
    request_with(by_skill)
}

pub(super) fn dict(body: &str) -> HookResponse {
    let Value::Object(fields) = json(body) else {
        panic!("a dict response is a JSON object");
    };
    HookResponse::Dict(fields)
}

pub(super) fn tool_use(marker: &str, id: &str, name: &str, input: &str) -> HookResponse {
    dict(&format!(
        r#"{{"marker":"{marker}","stop_reason":"tool_use","content":[{{"type":"tool_use","id":"{id}","name":"{name}","input":{input}}}]}}"#
    ))
}

pub(super) fn final_answer(marker: &str) -> HookResponse {
    dict(&format!(
        r#"{{"marker":"{marker}","stop_reason":"end_turn","content":[{{"type":"text","text":"done"}}]}}"#
    ))
}

pub(super) fn object_response(content: Vec<ContentBlock>, stop_reason: &str) -> HookResponse {
    HookResponse::Object(ObjectResponse {
        content,
        choices: Vec::new(),
        stop_reason: Some(stop_reason.to_owned()),
        generated_files: GeneratedFilesAttribute::Writable(Vec::new()),
        model_extra: Some(serde_json::Map::new()),
    })
}

pub(super) fn tool_block(id: &str, name: &str, input: &str) -> ContentBlock {
    ContentBlock {
        kind: "tool_use".to_owned(),
        id: Some(id.to_owned()),
        name: Some(name.to_owned()),
        input: json(input),
        text: None,
    }
}

pub(super) fn chat_tool_call(arguments: &str) -> HookResponse {
    let escaped = serde_json::to_string(arguments).unwrap();
    let body = format!(
        r#"{{"id":"chatcmpl-1","object":"chat.completion","created":1,"model":"gpt-4o","choices":[{{"index":0,"message":{{"role":"assistant","content":null,"tool_calls":[{{"id":"call_1","type":"function","function":{{"name":"litellm_code_execution","arguments":{escaped}}}}}]}},"finish_reason":"tool_calls"}}]}}"#
    );
    let parsed: ChatResponse = serde_json::from_str(&body).unwrap();
    HookResponse::Object(ObjectResponse {
        content: Vec::new(),
        choices: parsed.choices,
        stop_reason: Some("tool_use".to_owned()),
        generated_files: GeneratedFilesAttribute::Rejected,
        model_extra: None,
    })
}

pub(super) fn generated_names(response: &HookResponse) -> Vec<String> {
    let HookResponse::Dict(fields) = response else {
        panic!("the test expects a dict response");
    };
    let Some(Value::Array(files)) = fields.get("_litellm_generated_files") else {
        return Vec::new();
    };
    files
        .iter()
        .map(|file| file["name"].as_str().unwrap().to_owned())
        .collect()
}

pub(super) fn marker(response: &HookResponse) -> String {
    let HookResponse::Dict(fields) = response else {
        panic!("the test expects a dict response");
    };
    fields["marker"].as_str().unwrap().to_owned()
}

#[derive(Clone, Default)]
pub(super) struct CapturedLogs(Arc<Mutex<Vec<(Level, String)>>>);

impl CapturedLogs {
    pub(super) fn entries(&self) -> Vec<(Level, String)> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn messages(&self) -> Vec<String> {
        self.entries()
            .into_iter()
            .map(|(_, message)| message)
            .collect()
    }
}

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for CapturedLogs {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let mut message = Message::default();
        event.record(&mut message);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((*event.metadata().level(), message.0));
    }
}

pub(super) fn capture() -> (CapturedLogs, DefaultGuard) {
    let logs = CapturedLogs::default();
    let guard = set_default(registry().with(logs.clone()));
    (logs, guard)
}

pub(super) async fn run_loop(
    hook: &SkillsInjectionHook<(), ScriptedSandbox>,
    response: HookResponse,
    replies: Vec<Reply>,
) -> (Result<HookResponse, SkillsLoopError>, ScriptedContinuation) {
    let continuation = ScriptedContinuation::new(replies);
    let files = skill_files(&[("core/gif_builder.py", "class GifBuilder: pass")]);
    let result = hook
        .execute_messages_loop(&request(), response, &files, &continuation)
        .await;
    (result, continuation)
}

pub(super) fn results_of(continuation: &ScriptedContinuation, index: usize) -> Value {
    let requests = continuation.requests();
    let last = requests[index].messages.last().unwrap();
    last["content"].clone()
}

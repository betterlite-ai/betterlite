use std::collections::VecDeque;
use std::fmt::Debug;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_core::api::skills::continuation::messages::{
    ContinuationRequest, MessagesContinuation,
};
use betterlite_core::api::skills::continuation::request::{RequestMetadata, SkillsRequestData};
use betterlite_core::api::skills::hook::SkillsInjectionHook;
use betterlite_core::api::skills::hook::execution::{SandboxOutcome, SkillFiles, SkillsSandbox};
use betterlite_core::api::skills::hook::response::HookResponse;
use betterlite_core::api::skills::hook::settings::HookSettings;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use crate::helpers::{Helper, HelperError, HelperResult};

// What LiteLLM's SkillsSandboxExecutor answers when llm-sandbox is not installed, as in the oracle's Python
const SANDBOX_MISSING: &str = "llm-sandbox not installed. Install `llm-sandbox`.";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    max_iterations: u32,
    enabled: bool,
    model: String,
    messages: Vec<Value>,
    tools: Vec<Value>,
    #[serde(default)]
    max_tokens: Option<u64>,
    skill_files: IndexMap<String, IndexMap<String, String>>,
    response: Map<String, Value>,
    replies: Vec<Reply>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Reply {
    Response {
        response: Map<String, Value>,
    },
    #[serde(rename = "none")]
    Absent,
    Error {
        message: String,
    },
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
struct ScriptFailure {
    message: String,
}

struct MissingSandbox;

impl SkillsSandbox for MissingSandbox {
    type Error = std::convert::Infallible;

    fn execute(
        &self,
        _code: &str,
        _skill_files: &SkillFiles,
    ) -> impl Future<Output = Result<SandboxOutcome, Self::Error>> + Send {
        ready(Ok(SandboxOutcome {
            output: Some(String::new()),
            error: Some(SANDBOX_MISSING.to_owned()),
            files: Vec::new(),
        }))
    }
}

struct ScriptedContinuation {
    replies: Mutex<VecDeque<Reply>>,
    calls: Mutex<Vec<Value>>,
}

impl MessagesContinuation for ScriptedContinuation {
    type Error = ScriptFailure;

    fn create(
        &self,
        request: &ContinuationRequest,
    ) -> impl Future<Output = Result<Option<HookResponse>, ScriptFailure>> + Send {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({
                "model": request.model,
                "messages": request.messages,
                "tools": request.tools,
                "max_tokens": request.max_tokens,
            }));
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        ready(match reply {
            Some(Reply::Response { response }) => Ok(Some(HookResponse::Dict(response))),
            Some(Reply::Absent) => Ok(None),
            Some(Reply::Error { message }) => Err(ScriptFailure { message }),
            None => Err(ScriptFailure {
                message: "the scripted replies are used up".to_owned(),
            }),
        })
    }
}

#[derive(Clone, Default)]
struct Lines(Arc<Mutex<Vec<Value>>>);

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Lines {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let mut message = Message::default();
        event.record(&mut message);
        let level = match *event.metadata().level() {
            Level::ERROR => "ERROR",
            Level::WARN => "WARNING",
            Level::INFO => "INFO",
            Level::DEBUG => "DEBUG",
            Level::TRACE => "TRACE",
        };
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({"level": level, "message": message.0}));
    }
}

fn failure(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn request_data(input: &Input) -> SkillsRequestData {
    let skill_files = input
        .skill_files
        .iter()
        .map(|(skill_id, paths)| {
            let files: SkillFiles = paths
                .iter()
                .map(|(path, text)| (path.clone(), text.as_bytes().to_vec()))
                .collect();
            (skill_id.clone(), files)
        })
        .collect();
    SkillsRequestData {
        model: input.model.clone(),
        messages: input.messages.clone(),
        tools: input.tools.clone(),
        max_tokens: input.max_tokens,
        skills: RequestMetadata {
            code_execution_enabled: input.enabled,
            skill_files,
        },
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| failure("RuntimeError", error.to_string()))?;
        let hook = SkillsInjectionHook {
            prompt_handler: (),
            executor: MissingSandbox,
            settings: HookSettings {
                max_iterations: input.max_iterations,
                sandbox_timeout: Duration::from_secs(120),
            },
        };
        let continuation = ScriptedContinuation {
            replies: Mutex::new(VecDeque::from(input.replies.clone())),
            calls: Mutex::new(Vec::new()),
        };
        let lines = Lines::default();
        let guard =
            tracing::subscriber::set_default(tracing_subscriber::registry().with(lines.clone()));
        let outcome = runtime.block_on(hook.post_call_success(
            &request_data(input),
            HookResponse::Dict(input.response.clone()),
            &continuation,
        ));
        drop(guard);
        let result = match outcome.map_err(|error| failure("SkillsLoopError", error.to_string()))? {
            None => Value::Null,
            Some(HookResponse::Dict(fields)) => Value::Object(fields),
            Some(HookResponse::Object(_)) => {
                return Err(failure(
                    "UnexpectedResponse",
                    "a dict response came back as an object response".to_owned(),
                ));
            }
        };
        let calls = continuation
            .calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let logs = lines
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        Ok(json!({
            "result": result,
            "continuation_calls": calls,
            "logs": logs,
        }))
    }
}

#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::future::{self, Future};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{StreamExt as _, stream};
use rstest::rstest;
use rust_decimal::Decimal;
use tempfile::{Builder, TempDir};
use tokio::io::{Empty, Sink};

use betterlite_harness::context::{SessionContext, SessionSettings, TurnOutcome};
use betterlite_harness::deepagents::sandbox_backend::TOOL_FILTER_NAME;
use betterlite_harness::deepagents::sandbox_backend::usage::{
    Generation, LlmResult, TokenPricer, UsageCallback,
};
use betterlite_harness::deepagents::transformation::config::DeepAgentsHarnessConfig;
use betterlite_harness::deepagents::transformation::message::{
    Content, Message, MessageKind, MessageUsage, ToolCall, content_text,
};
use betterlite_harness::deepagents::transformation::update::{
    ActionRequest, HitlDecision, INTERRUPT_KEY, Interrupt, InterruptValue, Update, UpdateEntry,
};
use betterlite_harness::deepagents::transformation::{
    BASE_RECURSION_LIMIT, DEEPAGENTS_STEPS_PER_TURN, DEFAULT_RECURSION_LIMIT, INSTALL_HINT,
};
use betterlite_harness::errors::HarnessError;
use betterlite_harness::events::{Approval, Decision, Event};
use betterlite_harness::handlers::BaseHarnessHandler as _;
use betterlite_harness::options::{DeepAgentsOptions, HarnessOptions};
use betterlite_harness::sandbox::{CompletedRun, Process, Sandbox, SandboxCommand};
use betterlite_harness::types::{Harness, JsonObject, PermissionMode, Usage};

use crate::sandbox_backend::Permissions;

use super::DeepAgentsHandler;
use super::graph::{
    AgentRuntime, AgentSpec, Graph, GraphInput, GraphState, GraphStream, MissingRuntime, RunConfig,
    StreamPart,
};

const GENERAL_NAME: &str = "general-purpose";

fn json_object(text: &str) -> JsonObject {
    serde_json::from_str(text).unwrap()
}

fn tempdir() -> TempDir {
    Builder::new()
        .prefix("betterlite-deepagents-handler-test-")
        .tempdir()
        .unwrap()
}

struct NoProcess {
    stdout: Empty,
    stderr: Empty,
}

impl Process for NoProcess {
    type Stdin = Sink;
    type Stdout = Empty;
    type Stderr = Empty;

    fn stdin(&mut self) -> Option<&mut Sink> {
        None
    }

    fn close_stdin(&mut self) {}

    fn stdout(&mut self) -> &mut Empty {
        &mut self.stdout
    }

    fn stderr(&mut self) -> &mut Empty {
        &mut self.stderr
    }

    fn wait(&mut self) -> impl Future<Output = Result<i32, HarnessError>> + Send {
        future::ready(Ok(0_i32))
    }

    fn kill(&mut self) -> impl Future<Output = Result<(), HarnessError>> + Send {
        future::ready(Ok(()))
    }
}

struct MemorySandbox {
    workdir: String,
    writes: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
}

impl Sandbox for MemorySandbox {
    type Process = NoProcess;

    fn workdir(&self) -> &str {
        &self.workdir
    }

    fn exec(
        &self,
        _command: &SandboxCommand,
    ) -> impl Future<Output = Result<NoProcess, HarnessError>> + Send {
        future::ready(Err(HarnessError::SessionClosed))
    }

    fn run(
        &self,
        _command: &SandboxCommand,
        _timeout: Duration,
        _max_output_bytes: usize,
    ) -> impl Future<Output = Result<CompletedRun, HarnessError>> + Send {
        future::ready(Err(HarnessError::SessionClosed))
    }

    fn read(
        &self,
        _path: &str,
        _max_bytes: usize,
    ) -> impl Future<Output = Result<Vec<u8>, HarnessError>> + Send {
        future::ready(Err(HarnessError::SessionClosed))
    }

    fn write(
        &self,
        path: &str,
        data: &[u8],
    ) -> impl Future<Output = Result<(), HarnessError>> + Send {
        self.writes
            .lock()
            .unwrap()
            .insert(path.to_owned(), data.to_vec());
        future::ready(Ok(()))
    }

    fn host_url(&self, port: u16) -> String {
        format!("http://127.0.0.1:{port}")
    }

    fn which(
        &self,
        _binary: &str,
    ) -> impl Future<Output = Result<Option<String>, HarnessError>> + Send {
        future::ready(Ok(None))
    }

    fn snapshot(
        &self,
    ) -> impl Future<Output = Result<BTreeMap<String, String>, HarnessError>> + Send {
        future::ready(Ok(BTreeMap::new()))
    }

    fn tempdir(&self) -> impl Future<Output = Result<String, HarnessError>> + Send {
        future::ready(Err(HarnessError::SessionClosed))
    }

    fn close(&self) -> impl Future<Output = Result<(), HarnessError>> + Send {
        future::ready(Ok(()))
    }
}

fn context(workdir: &str) -> SessionContext<MemorySandbox> {
    let mut settings = SessionSettings::default();
    settings.model = Some("openai/gpt-4o".to_owned());
    SessionContext {
        harness: Harness::DeepAgents,
        sandbox: MemorySandbox {
            workdir: workdir.to_owned(),
            writes: Arc::new(Mutex::new(BTreeMap::new())),
        },
        session_id: "session-1".to_owned(),
        settings,
        outcome: TurnOutcome::default(),
    }
}

#[derive(Default)]
struct Plan {
    passes: Mutex<VecDeque<Vec<Result<StreamPart, HarnessError>>>>,
    state: Mutex<GraphState>,
    specs: Mutex<Vec<AgentSpec>>,
    cost_models: Mutex<Vec<Option<String>>>,
    inputs: Mutex<Vec<GraphInput>>,
    run_configs: Mutex<Vec<RunConfig>>,
    state_configs: Mutex<Vec<RunConfig>>,
    usage_per_call: Mutex<Option<LlmResult>>,
}

struct FlatPrice;

impl TokenPricer for FlatPrice {
    fn cost_per_token(
        &self,
        _model: &str,
        prompt_tokens: u64,
        completion_tokens: u64,
    ) -> Option<Decimal> {
        Some(Decimal::from(prompt_tokens.checked_add(completion_tokens)?))
    }
}

struct FakeGraph {
    plan: Arc<Plan>,
}

impl Graph for FakeGraph {
    fn stream(&self, input: GraphInput, config: RunConfig) -> GraphStream {
        self.plan.inputs.lock().unwrap().push(input);
        let usage = self.plan.usage_per_call.lock().unwrap().clone();
        if let (Some(result), Some(callback)) = (usage.as_ref(), config.usage_callback.as_ref()) {
            callback.record(result);
        }
        self.plan.run_configs.lock().unwrap().push(config);
        let parts = self
            .plan
            .passes
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_default();
        Box::pin(stream::iter(parts))
    }

    fn state(&self, config: RunConfig) -> GraphState {
        self.plan.state_configs.lock().unwrap().push(config);
        let state = self.plan.state.lock().unwrap();
        GraphState {
            messages: state.messages.clone(),
            structured_response: state.structured_response.clone(),
        }
    }
}

struct FakeRuntime {
    plan: Arc<Plan>,
}

impl FakeRuntime {
    fn new() -> (Self, Arc<Plan>) {
        let plan = Arc::new(Plan::default());
        (
            Self {
                plan: Arc::clone(&plan),
            },
            plan,
        )
    }
}

impl AgentRuntime for FakeRuntime {
    type Graph = FakeGraph;

    fn build_agent(&self, spec: AgentSpec) -> Result<FakeGraph, HarnessError> {
        self.plan.specs.lock().unwrap().push(spec);
        Ok(FakeGraph {
            plan: Arc::clone(&self.plan),
        })
    }

    fn general_subagent(&self) -> JsonObject {
        json_object(r#"{"name": "general-purpose", "description": "the default agent"}"#)
    }

    fn openai_messages(&self, messages: &[Message]) -> Vec<JsonObject> {
        messages
            .iter()
            .map(|message| {
                let mut converted = JsonObject::new();
                converted.insert(
                    "content".to_owned(),
                    serde_json::to_value(content_text(&message.content)).unwrap(),
                );
                converted
            })
            .collect()
    }

    fn usage_callback(&self, cost_model: Option<String>) -> Option<UsageCallback> {
        self.plan
            .cost_models
            .lock()
            .unwrap()
            .push(cost_model.clone());
        Some(UsageCallback::new(cost_model, Arc::new(FlatPrice)))
    }
}

fn handler(runtime: FakeRuntime) -> DeepAgentsHandler<DeepAgentsHarnessConfig, FakeRuntime> {
    DeepAgentsHandler {
        config: DeepAgentsHarnessConfig,
        runtime,
        graph: None,
        thread_id: None,
        skip_tools: BTreeSet::new(),
    }
}

fn built_spec(plan: &Plan) -> AgentSpec {
    plan.specs.lock().unwrap().first().unwrap().clone()
}

fn plain_message(kind: MessageKind, text: &str) -> Message {
    Message {
        kind,
        content: Content::Text(text.to_owned()),
        reasoning_content: String::new(),
        tool_calls: Vec::new(),
        tool_call_id: String::new(),
        name: None,
        is_error: false,
        usage: MessageUsage::default(),
        reported_cost: None,
    }
}

fn ai(text: &str) -> Message {
    plain_message(MessageKind::Ai, text)
}

fn human(text: &str) -> Message {
    plain_message(MessageKind::Human, text)
}

fn text_part(text: &str, node: &str) -> Result<StreamPart, HarnessError> {
    Ok(StreamPart::Message {
        message: ai(text),
        node: node.to_owned(),
    })
}

fn update_part(update: Update) -> Result<StreamPart, HarnessError> {
    Ok(StreamPart::Update(update))
}

fn update_with(entries: Vec<(&str, UpdateEntry)>) -> Update {
    Update {
        entries: entries
            .into_iter()
            .map(|(node, entry)| (node.to_owned(), entry))
            .collect(),
    }
}

fn interrupt_update(id: &str, requests: &[(&str, &str)]) -> Update {
    let action_requests = requests
        .iter()
        .map(|(name, args)| ActionRequest {
            name: (*name).to_owned(),
            args: json_object(args),
        })
        .collect();
    let interrupt = Interrupt {
        id: id.to_owned(),
        value: InterruptValue { action_requests },
    };
    update_with(vec![(
        INTERRUPT_KEY,
        UpdateEntry::Interrupts(vec![interrupt]),
    )])
}

fn deltas(events: &[Event]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::Text { delta } => Some(delta.clone()),
            _ => None,
        })
        .collect()
}

fn approvals(events: &[Event]) -> Vec<&Approval> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::Approval(approval) => Some(approval),
            _ => None,
        })
        .collect()
}

async fn run_turn<A, S>(
    handler: &mut DeepAgentsHandler<DeepAgentsHarnessConfig, FakeRuntime>,
    context: &mut SessionContext<S>,
    prompt: &str,
    answer: A,
) -> Vec<Event>
where
    A: Fn(&Approval) -> Decision,
    S: Sandbox,
{
    let mut stream = Box::pin(handler.turn(context, prompt));
    let mut events = Vec::new();
    while let Some(item) = stream.next().await {
        let event = item.unwrap();
        if let Event::Approval(approval) = &event {
            approval.decision.set(answer(approval)).unwrap();
        }
        events.push(event);
    }
    events
}

#[rstest]
#[case::ask_with_no_disabled_tools(PermissionMode::Ask, &[], true, true)]
#[case::read_only(PermissionMode::ReadOnly, &[], false, false)]
#[case::edit(PermissionMode::Edit, &[], true, false)]
#[case::full_with_write_disabled(PermissionMode::Full, &["write"], false, true)]
#[case::full_with_bash_disabled(PermissionMode::Full, &["bash"], true, false)]
#[case::full_with_read_tool_disabled(PermissionMode::Full, &["ls"], true, true)]
#[tokio::test]
async fn build_computes_backend_permissions(
    #[case] permissions: PermissionMode,
    #[case] disabled: &[&str],
    #[case] writable: bool,
    #[case] allow_execute: bool,
) {
    let (runtime, plan) = FakeRuntime::new();
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    ctx.settings.permissions = permissions;
    ctx.settings.disable_tools = disabled.iter().map(|name| (*name).to_owned()).collect();
    handler.start(&mut ctx).await.unwrap();
    let spec = built_spec(&plan);
    assert_eq!(
        spec.permissions,
        Permissions {
            writable,
            allow_execute
        }
    );
}

#[rstest]
#[case::nothing_blocked(PermissionMode::Ask, &[], false)]
#[case::read_only(PermissionMode::ReadOnly, &[], true)]
#[case::one_disabled_tool(PermissionMode::Full, &["ls"], true)]
#[tokio::test]
async fn build_passes_a_tool_filter_only_when_tools_are_blocked(
    #[case] permissions: PermissionMode,
    #[case] disabled: &[&str],
    #[case] has_filter: bool,
) {
    let (runtime, plan) = FakeRuntime::new();
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    ctx.settings.permissions = permissions;
    ctx.settings.disable_tools = disabled.iter().map(|name| (*name).to_owned()).collect();
    handler.start(&mut ctx).await.unwrap();
    let spec = built_spec(&plan);
    assert_eq!(spec.middleware.is_some(), has_filter);
}

#[tokio::test]
async fn build_the_tool_filter_lists_the_blocked_tools() {
    let (runtime, plan) = FakeRuntime::new();
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    ctx.settings.permissions = PermissionMode::ReadOnly;
    handler.start(&mut ctx).await.unwrap();
    let spec = built_spec(&plan);
    let filter = spec.middleware.unwrap();
    let blocked: Vec<&str> = filter.blocked.iter().map(String::as_str).collect();
    assert_eq!(blocked, ["delete", "edit_file", "execute", "write_file"]);
}

#[rstest]
#[case::ask(PermissionMode::Ask, true)]
#[case::full(PermissionMode::Full, false)]
#[tokio::test]
async fn build_interrupts_only_sessions_that_ask(
    #[case] permissions: PermissionMode,
    #[case] has_policy: bool,
) {
    let (runtime, plan) = FakeRuntime::new();
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    ctx.settings.permissions = permissions;
    handler.start(&mut ctx).await.unwrap();
    let spec = built_spec(&plan);
    let policy = spec.interrupt_on.as_ref();
    assert_eq!(policy.is_some(), has_policy);
    if let Some(policy) = policy {
        let tools: Vec<&str> = policy.keys().map(String::as_str).collect();
        assert_eq!(tools, ["delete", "edit_file", "execute", "write_file"]);
    }
}

#[tokio::test]
async fn build_carries_the_model_instructions_policy_and_schema() {
    let (runtime, plan) = FakeRuntime::new();
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    ctx.settings.instructions = Some("be helpful".to_owned());
    ctx.settings.permissions = PermissionMode::Ask;
    ctx.settings.output_schema = Some(json_object(r#"{"title":"Answer","type":"object"}"#));
    handler.start(&mut ctx).await.unwrap();
    let spec = built_spec(&plan);
    assert_eq!(spec.chat_model.model, "openai/gpt-4o");
    assert!(spec.chat_model.api_base.is_none());
    assert!(spec.chat_model.api_key.is_none());
    assert!(spec.chat_model.extra_headers.is_none());
    assert_eq!(spec.system_prompt.as_deref(), Some("be helpful"));
    assert_eq!(
        spec.output_schema,
        Some(json_object(r#"{"title":"Answer","type":"object"}"#))
    );
    assert!(spec.interrupt_on.is_some());
    assert_eq!(handler.skip_tools, BTreeSet::from(["Answer".to_owned()]));
}

#[tokio::test]
async fn build_defaults_the_thread_id_and_keeps_it_on_a_second_start() {
    let (runtime, plan) = FakeRuntime::new();
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    handler.start(&mut ctx).await.unwrap();
    assert_eq!(handler.thread_id.as_deref(), Some("session-1"));
    handler.start(&mut ctx).await.unwrap();
    assert_eq!(handler.thread_id.as_deref(), Some("session-1"));
    assert!(handler.graph.is_some());
    assert_eq!(plan.specs.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn build_resume_replaces_the_thread_id() {
    let (runtime, _plan) = FakeRuntime::new();
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    handler.resume(&mut ctx, "native-7").await.unwrap();
    assert_eq!(handler.thread_id.as_deref(), Some("native-7"));
    assert_eq!(handler.native_session_id().as_deref(), Some("native-7"));
}

#[tokio::test]
async fn build_stop_twice_is_fine() {
    let (runtime, _plan) = FakeRuntime::new();
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    handler.start(&mut ctx).await.unwrap();
    handler.stop(&mut ctx).await.unwrap();
    assert!(handler.graph.is_none());
    handler.stop(&mut ctx).await.unwrap();
}

#[tokio::test]
async fn build_fails_with_the_install_hint_when_the_runtime_is_missing() {
    let mut handler = DeepAgentsHandler {
        config: DeepAgentsHarnessConfig,
        runtime: MissingRuntime,
        graph: None,
        thread_id: None,
        skip_tools: BTreeSet::new(),
    };
    let mut ctx = context("/work");
    let error = handler.start(&mut ctx).await.unwrap_err();
    assert!(matches!(error, HarnessError::InstallFailed(_)));
    assert!(error.to_string().contains(INSTALL_HINT));
    assert!(handler.graph.is_none());
    assert!(handler.thread_id.is_none());
}

#[tokio::test]
async fn build_adds_the_general_subagent_with_the_filter_when_tools_are_blocked() {
    let (runtime, plan) = FakeRuntime::new();
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    ctx.settings.permissions = PermissionMode::ReadOnly;
    handler.start(&mut ctx).await.unwrap();
    let spec = built_spec(&plan);
    assert_eq!(spec.subagents.len(), 1);
    let general = spec.subagents.first().unwrap();
    assert_eq!(
        general.get("name").and_then(|name| name.as_str()),
        Some(GENERAL_NAME)
    );
    let filter = general.get("middleware").unwrap().as_array().unwrap();
    assert_eq!(filter.len(), 1);
    let entry = filter.first().unwrap();
    assert_eq!(
        entry.get("name").and_then(|name| name.as_str()),
        Some(TOOL_FILTER_NAME)
    );
    let blocked: Vec<&str> = entry
        .get("blocked")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool.as_str().unwrap())
        .collect();
    assert_eq!(blocked, ["delete", "edit_file", "execute", "write_file"]);
}

#[tokio::test]
async fn build_keeps_the_general_subagent_out_when_nothing_is_blocked() {
    let (runtime, plan) = FakeRuntime::new();
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    handler.start(&mut ctx).await.unwrap();
    let spec = built_spec(&plan);
    assert!(spec.subagents.is_empty());
    assert!(spec.middleware.is_none());
}

#[tokio::test]
async fn build_keeps_the_users_general_subagent_unchanged() {
    let (runtime, plan) = FakeRuntime::new();
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    ctx.settings.permissions = PermissionMode::ReadOnly;
    let user = json_object(r#"{"name": "general-purpose", "description": "mine"}"#);
    ctx.settings.options = Some(HarnessOptions::DeepAgents(DeepAgentsOptions {
        subagents: vec![user.clone()],
        recursion_limit: None,
    }));
    handler.start(&mut ctx).await.unwrap();
    let spec = built_spec(&plan);
    assert_eq!(spec.subagents, vec![user]);
}

#[tokio::test]
async fn build_puts_user_subagents_after_the_general_one() {
    let (runtime, plan) = FakeRuntime::new();
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    ctx.settings.disable_tools = vec!["bash".to_owned()];
    let user = json_object(r#"{"name": "researcher", "description": "finds things"}"#);
    ctx.settings.options = Some(HarnessOptions::DeepAgents(DeepAgentsOptions {
        subagents: vec![user.clone()],
        recursion_limit: None,
    }));
    handler.start(&mut ctx).await.unwrap();
    let spec = built_spec(&plan);
    assert_eq!(spec.subagents.len(), 2);
    assert_eq!(
        spec.subagents
            .first()
            .unwrap()
            .get("name")
            .and_then(|name| name.as_str()),
        Some(GENERAL_NAME)
    );
    assert_eq!(spec.subagents.get(1), Some(&user));
}

#[tokio::test]
async fn build_copies_skills_under_the_workdir_and_passes_the_virtual_dir() {
    let (runtime, plan) = FakeRuntime::new();
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    let skill = tempdir();
    let skill_dir = skill.path().join("my-skill");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(skill_dir.join("SKILL.md"), "# skill").unwrap();
    ctx.settings.skills = vec![skill_dir.to_str().unwrap().to_owned()];
    handler.start(&mut ctx).await.unwrap();
    let spec = built_spec(&plan);
    assert_eq!(spec.skills, Some(vec!["/.deepagents/skills/".to_owned()]));
    let writes = ctx.sandbox.writes.lock().unwrap();
    assert_eq!(writes.len(), 1);
    let (path, data) = writes.first_key_value().unwrap();
    assert_eq!(path, "/work/.deepagents/skills/my-skill/SKILL.md");
    assert_eq!(data.as_slice(), b"# skill");
}

#[tokio::test]
async fn build_without_skills_passes_none_and_copies_nothing() {
    let (runtime, plan) = FakeRuntime::new();
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    handler.start(&mut ctx).await.unwrap();
    let spec = built_spec(&plan);
    assert!(spec.skills.is_none());
    assert!(ctx.sandbox.writes.lock().unwrap().is_empty());
}

#[tokio::test]
async fn history_returns_the_runtime_openai_messages() {
    let (runtime, plan) = FakeRuntime::new();
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    handler.start(&mut ctx).await.unwrap();
    plan.state.lock().unwrap().messages = vec![human("hi"), ai("done")];
    let messages = handler.history(&ctx).await.unwrap();
    assert_eq!(
        messages,
        vec![
            json_object(r#"{"content":"hi"}"#),
            json_object(r#"{"content":"done"}"#)
        ]
    );
    let configs = plan.state_configs.lock().unwrap();
    let config = configs.first().unwrap();
    assert_eq!(config.thread_id, "session-1");
    assert_eq!(config.recursion_limit, DEFAULT_RECURSION_LIMIT);
    assert!(config.usage_callback.is_none());
}

#[tokio::test]
async fn history_returns_an_empty_list_when_the_state_has_none() {
    let (runtime, _plan) = FakeRuntime::new();
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    handler.start(&mut ctx).await.unwrap();
    assert!(handler.history(&ctx).await.unwrap().is_empty());
}

#[tokio::test]
async fn history_fails_with_session_closed_before_start() {
    let (runtime, _plan) = FakeRuntime::new();
    let handler = handler(runtime);
    let ctx = context("/work");
    let error = handler.history(&ctx).await.unwrap_err();
    assert!(matches!(error, HarnessError::SessionClosed));
}

#[tokio::test]
async fn turn_yields_only_model_node_messages() {
    let (runtime, plan) = FakeRuntime::new();
    plan.passes.lock().unwrap().push_back(vec![
        text_part("hello", "model"),
        text_part("echo", "tools"),
    ]);
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    handler.start(&mut ctx).await.unwrap();
    let events = run_turn(&mut handler, &mut ctx, "hi", |_| Decision::Allow).await;
    assert_eq!(deltas(&events), ["hello"]);
}

fn answer_and_read_pass() -> Vec<Result<StreamPart, HarnessError>> {
    let mut called = ai("thinking");
    called.tool_calls = vec![
        ToolCall {
            id: "1".to_owned(),
            name: "Answer".to_owned(),
            args: JsonObject::new(),
        },
        ToolCall {
            id: "2".to_owned(),
            name: "read_file".to_owned(),
            args: json_object(r#"{"path":"a.txt"}"#),
        },
    ];
    let mut result = ai("{}");
    result.kind = MessageKind::Tool;
    result.name = Some("Answer".to_owned());
    result.tool_call_id = "1".to_owned();
    vec![update_part(update_with(vec![
        ("model", UpdateEntry::Messages(vec![called])),
        ("tools", UpdateEntry::Messages(vec![result])),
    ]))]
}

#[tokio::test]
async fn turn_yields_update_events_without_the_skipped_tools() {
    let (runtime, plan) = FakeRuntime::new();
    plan.passes
        .lock()
        .unwrap()
        .push_back(answer_and_read_pass());
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    ctx.settings.output_schema = Some(json_object(r#"{"title":"Answer","type":"object"}"#));
    handler.start(&mut ctx).await.unwrap();
    let events = run_turn(&mut handler, &mut ctx, "hi", |_| Decision::Allow).await;
    let calls: Vec<(&str, &str)> = events
        .iter()
        .filter_map(|event| match event {
            Event::ToolCall {
                name, native_name, ..
            } => Some((name.as_str(), native_name.as_str())),
            _ => None,
        })
        .collect();
    assert_eq!(calls, [("read", "read_file")]);
}

#[tokio::test]
async fn turn_passes_thread_id_recursion_limit_and_usage_callback() {
    let (runtime, plan) = FakeRuntime::new();
    plan.passes
        .lock()
        .unwrap()
        .push_back(vec![text_part("done", "model")]);
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    ctx.settings.max_turns = Some(2);
    handler.start(&mut ctx).await.unwrap();
    run_turn(&mut handler, &mut ctx, "hi", |_| Decision::Allow).await;
    let configs = plan.run_configs.lock().unwrap();
    let config = configs.first().unwrap();
    assert_eq!(config.thread_id, "session-1");
    let expected_limit =
        BASE_RECURSION_LIMIT.saturating_add(2_u32.saturating_mul(DEEPAGENTS_STEPS_PER_TURN));
    assert_eq!(config.recursion_limit, expected_limit);
    assert!(config.usage_callback.is_some());
    let cost_models = plan.cost_models.lock().unwrap();
    assert_eq!(cost_models.len(), 1);
    assert_eq!(
        cost_models.first().unwrap().as_deref(),
        Some("openai/gpt-4o")
    );
    let state_configs = plan.state_configs.lock().unwrap();
    assert_eq!(state_configs.first().unwrap().thread_id, "session-1");
    assert!(state_configs.first().unwrap().usage_callback.is_some());
}

#[tokio::test]
async fn turn_uses_the_resumed_thread_id() {
    let (runtime, plan) = FakeRuntime::new();
    plan.passes
        .lock()
        .unwrap()
        .push_back(vec![text_part("done", "model")]);
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    handler.start(&mut ctx).await.unwrap();
    handler.resume(&mut ctx, "native-7").await.unwrap();
    run_turn(&mut handler, &mut ctx, "hi", |_| Decision::Allow).await;
    assert_eq!(
        plan.run_configs.lock().unwrap().first().unwrap().thread_id,
        "native-7"
    );
}

#[tokio::test]
async fn turn_sets_final_text_and_structured_output() {
    let (runtime, plan) = FakeRuntime::new();
    plan.passes
        .lock()
        .unwrap()
        .push_back(vec![text_part("the answer", "model")]);
    {
        let mut state = plan.state.lock().unwrap();
        state.messages = vec![human("hi"), ai("the answer")];
        state.structured_response = Some(json_object(r#"{"result":7}"#));
    }
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    ctx.settings.output_schema = Some(json_object(r#"{"title":"Answer","type":"object"}"#));
    handler.start(&mut ctx).await.unwrap();
    run_turn(&mut handler, &mut ctx, "hi", |_| Decision::Allow).await;
    assert_eq!(ctx.outcome.final_text, "the answer");
    assert_eq!(ctx.outcome.output_json, Some(r#"{"result":7}"#.to_owned()));
}

#[tokio::test]
async fn turn_leaves_output_json_empty_without_an_output_schema() {
    let (runtime, plan) = FakeRuntime::new();
    plan.passes
        .lock()
        .unwrap()
        .push_back(vec![text_part("the answer", "model")]);
    plan.state.lock().unwrap().structured_response = Some(json_object(r#"{"result":7}"#));
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    handler.start(&mut ctx).await.unwrap();
    run_turn(&mut handler, &mut ctx, "hi", |_| Decision::Allow).await;
    assert_eq!(ctx.outcome.final_text, "the answer");
    assert!(ctx.outcome.output_json.is_none());
}

async fn run_write_then_bash_turn() -> (Arc<Plan>, Vec<Event>) {
    let (runtime, plan) = FakeRuntime::new();
    let mut called = ai("checking files");
    called.tool_calls = vec![ToolCall {
        id: "1".to_owned(),
        name: "write_file".to_owned(),
        args: json_object(r#"{"path":"a.txt"}"#),
    }];
    let mut interrupted = interrupt_update(
        "int-1",
        &[("write_file", r#"{"path":"a.txt"}"#), ("execute", "{}")],
    );
    interrupted
        .entries
        .insert("model".to_owned(), UpdateEntry::Messages(vec![called]));
    plan.passes
        .lock()
        .unwrap()
        .push_back(vec![update_part(interrupted)]);
    plan.passes
        .lock()
        .unwrap()
        .push_back(vec![text_part("finished", "model")]);
    let answers = Arc::new(Mutex::new(VecDeque::from([
        Decision::Allow,
        Decision::Deny {
            reason: "no".to_owned(),
        },
    ])));
    let turn_answers = Arc::clone(&answers);
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    handler.start(&mut ctx).await.unwrap();
    let events = run_turn(&mut handler, &mut ctx, "do it", move |_| {
        turn_answers.lock().unwrap().pop_front().unwrap()
    })
    .await;
    (plan, events)
}

#[tokio::test]
async fn turn_asks_one_approval_per_action_with_the_normalized_tool_name() {
    let (_plan, events) = run_write_then_bash_turn().await;
    let asked = approvals(&events);
    assert_eq!(asked.len(), 2);
    assert_eq!(asked.first().unwrap().tool, "write");
    assert_eq!(
        asked.first().unwrap().input,
        json_object(r#"{"path":"a.txt"}"#)
    );
    assert_eq!(asked.get(1).unwrap().tool, "bash");
    assert_eq!(asked.get(1).unwrap().input, JsonObject::new());
}

#[tokio::test]
async fn turn_resumes_with_the_decisions_in_order_under_the_interrupt_id() {
    let (plan, events) = run_write_then_bash_turn().await;
    assert_eq!(deltas(&events), ["finished"]);
    let inputs = plan.inputs.lock().unwrap();
    assert_eq!(inputs.len(), 2);
    assert!(matches!(
        inputs.first().unwrap(),
        GraphInput::Messages(message)
            if message.kind == MessageKind::Human && content_text(&message.content) == "do it"
    ));
    assert_eq!(
        inputs.get(1).unwrap(),
        &GraphInput::Resume(BTreeMap::from([(
            "int-1".to_owned(),
            vec![
                HitlDecision::Approve,
                HitlDecision::Reject {
                    message: "no".to_owned()
                }
            ]
        )]))
    );
}

#[tokio::test]
async fn turn_denies_with_the_default_message_when_the_reason_is_empty() {
    let (runtime, plan) = FakeRuntime::new();
    plan.passes
        .lock()
        .unwrap()
        .push_back(vec![update_part(interrupt_update(
            "int-1",
            &[("write_file", "{}")],
        ))]);
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    handler.start(&mut ctx).await.unwrap();
    run_turn(&mut handler, &mut ctx, "do it", |_| Decision::Deny {
        reason: String::new(),
    })
    .await;
    let inputs = plan.inputs.lock().unwrap();
    assert_eq!(
        inputs.get(1).unwrap(),
        &GraphInput::Resume(BTreeMap::from([(
            "int-1".to_owned(),
            vec![HitlDecision::Reject {
                message: "The user denied this tool call.".to_owned()
            }]
        )]))
    );
}

#[tokio::test]
async fn turn_repeats_until_a_pass_has_no_interrupts() {
    let (runtime, plan) = FakeRuntime::new();
    plan.passes
        .lock()
        .unwrap()
        .push_back(vec![update_part(interrupt_update(
            "int-1",
            &[("write_file", "{}")],
        ))]);
    plan.passes
        .lock()
        .unwrap()
        .push_back(vec![update_part(interrupt_update(
            "int-2",
            &[("execute", "{}")],
        ))]);
    plan.passes
        .lock()
        .unwrap()
        .push_back(vec![text_part("done", "model")]);
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    handler.start(&mut ctx).await.unwrap();
    let events = run_turn(&mut handler, &mut ctx, "do it", |_| Decision::Allow).await;
    assert_eq!(approvals(&events).len(), 2);
    assert_eq!(deltas(&events), ["done"]);
    let inputs = plan.inputs.lock().unwrap();
    assert_eq!(inputs.len(), 3);
    assert_eq!(
        inputs.get(1).unwrap(),
        &GraphInput::Resume(BTreeMap::from([(
            "int-1".to_owned(),
            vec![HitlDecision::Approve]
        )]))
    );
    assert_eq!(
        inputs.get(2).unwrap(),
        &GraphInput::Resume(BTreeMap::from([(
            "int-2".to_owned(),
            vec![HitlDecision::Approve]
        )]))
    );
}

#[tokio::test]
async fn turn_never_yields_done() {
    let (runtime, plan) = FakeRuntime::new();
    plan.passes
        .lock()
        .unwrap()
        .push_back(vec![text_part("done", "model")]);
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    handler.start(&mut ctx).await.unwrap();
    let events = run_turn(&mut handler, &mut ctx, "hi", |_| Decision::Allow).await;
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, Event::Done { .. }))
    );
}

#[tokio::test]
async fn turn_fails_with_session_closed_before_start_and_after_stop() {
    let (runtime, _plan) = FakeRuntime::new();
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    {
        let mut stream = Box::pin(handler.turn(&mut ctx, "hi"));
        let first = stream.next().await.unwrap();
        assert!(matches!(first, Err(HarnessError::SessionClosed)));
    }
    handler.start(&mut ctx).await.unwrap();
    handler.stop(&mut ctx).await.unwrap();
    let mut stream = Box::pin(handler.turn(&mut ctx, "again"));
    let first = stream.next().await.unwrap();
    assert!(matches!(first, Err(HarnessError::SessionClosed)));
}

fn one_generation(input_tokens: u64, output_tokens: u64) -> LlmResult {
    let mut called = ai("done");
    called.usage = MessageUsage {
        input_tokens,
        output_tokens,
    };
    LlmResult {
        generations: vec![vec![Generation {
            message: Some(called),
        }]],
    }
}

#[tokio::test]
async fn turn_adds_the_run_usage_to_the_outcome() {
    let (runtime, plan) = FakeRuntime::new();
    plan.passes
        .lock()
        .unwrap()
        .push_back(vec![text_part("done", "model")]);
    *plan.usage_per_call.lock().unwrap() = Some(one_generation(10, 5));
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    handler.start(&mut ctx).await.unwrap();
    run_turn(&mut handler, &mut ctx, "hi", |_| Decision::Allow).await;
    assert_eq!(
        ctx.outcome.usage,
        Usage {
            input_tokens: 10,
            output_tokens: 5,
            calls: 1,
        }
    );
    assert_eq!(ctx.outcome.cost, Decimal::new(15, 0));
}

#[tokio::test]
async fn turn_adds_each_turns_usage_to_the_outcome() {
    let (runtime, plan) = FakeRuntime::new();
    plan.passes
        .lock()
        .unwrap()
        .push_back(vec![text_part("done", "model")]);
    plan.passes
        .lock()
        .unwrap()
        .push_back(vec![text_part("done", "model")]);
    *plan.usage_per_call.lock().unwrap() = Some(one_generation(10, 5));
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    handler.start(&mut ctx).await.unwrap();
    run_turn(&mut handler, &mut ctx, "hi", |_| Decision::Allow).await;
    run_turn(&mut handler, &mut ctx, "again", |_| Decision::Allow).await;
    assert_eq!(
        ctx.outcome.usage,
        Usage {
            input_tokens: 20,
            output_tokens: 10,
            calls: 2,
        }
    );
    assert_eq!(ctx.outcome.cost, Decimal::new(30, 0));
}

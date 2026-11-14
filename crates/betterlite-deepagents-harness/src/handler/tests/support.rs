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
    let mut context = SessionContext {
        harness: Harness::DeepAgents,
        sandbox: MemorySandbox {
            workdir: workdir.to_owned(),
            writes: Arc::new(Mutex::new(BTreeMap::new())),
        },
        session_id: "session-1".to_owned(),
        settings: SessionSettings::default(),
        outcome: TurnOutcome::default(),
    };
    context.settings.model = Some("openai/gpt-4o".to_owned());
    context
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

    fn load_deps(&self) -> Result<(), HarnessError> {
        Ok(())
    }

    fn build_agent(&self, spec: AgentSpec) -> Result<FakeGraph, HarnessError> {
        let mut specs = self
            .plan
            .specs
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        specs.push(spec);
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

fn text_part(text: &str, node: &str) -> StreamPart {
    StreamPart::Message {
        message: Box::new(ai(text)),
        node: node.to_owned(),
    }
}

fn update_part(update: Update) -> StreamPart {
    StreamPart::Update(update)
}

fn text_pass(text: &str) -> Vec<Result<StreamPart, HarnessError>> {
    vec![Ok(text_part(text, "model"))]
}

fn interrupt_pass(id: &str, tool: &str) -> Vec<Result<StreamPart, HarnessError>> {
    vec![Ok(update_part(interrupt_update(id, &[(tool, "{}")])))]
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
        .filter_map(|event| {
            let Event::Text { delta } = event else {
                return None;
            };
            Some(delta.clone())
        })
        .collect()
}

fn approvals(events: &[Event]) -> Vec<&Approval> {
    let mut asked = Vec::new();
    for event in events {
        if let Event::Approval(approval) = event {
            asked.push(approval);
        }
    }
    asked
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

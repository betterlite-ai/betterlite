const MODEL: &str = "gpt-4o";
const CHAT_ID: &str = "chatcmpl-1";

const CONTENT_CHUNK: &str = r#"{"id":"chatcmpl-1","object":"chat.completion.chunk","created":1,"model":"gpt-4o","choices":[{"index":0,"delta":{"content":"TEXT"},"finish_reason":null}]}"#;
const CONTENT_USAGE_CHUNK: &str = r#"{"id":"chatcmpl-1","object":"chat.completion.chunk","created":1,"model":"gpt-4o","choices":[{"index":0,"delta":{"content":"Hi"},"finish_reason":null}],"usage":{"prompt_tokens":7,"completion_tokens":3,"total_tokens":10}}"#;
const FINISH_CHUNK: &str = r#"{"id":"chatcmpl-1","object":"chat.completion.chunk","created":1,"model":"gpt-4o","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#;
const USAGE_CHUNK: &str = r#"{"id":"chatcmpl-1","object":"chat.completion.chunk","created":1,"model":"gpt-4o","choices":[],"usage":{"prompt_tokens":7,"completion_tokens":3,"total_tokens":10}}"#;
const EMPTY_CHOICE_CHUNK: &str = r#"{"id":"chatcmpl-1","object":"chat.completion.chunk","created":1,"model":"gpt-4o","choices":[{"index":0,"delta":{},"finish_reason":null}]}"#;

const COMPLETE_RESPONSE: &str = r#"{"id":"chatcmpl-1","object":"chat.completion","created":1,"model":"gpt-4o","choices":[{"index":0,"message":{"role":"assistant","content":"Hello"},"finish_reason":"stop"}],"usage":{"prompt_tokens":7,"completion_tokens":3,"total_tokens":10},"service_tier":"default"}"#;
const COSTED_RESPONSE: &str = r#"{"id":"chatcmpl-1","object":"chat.completion","created":1,"model":"gpt-4o","choices":[{"index":0,"message":{"role":"assistant","content":"Hello"},"finish_reason":"stop"}],"usage":{"prompt_tokens":7,"completion_tokens":3,"total_tokens":10,"cost":0.01}}"#;

fn json(source: &str) -> JsonValue {
    from_str(source).unwrap()
}

fn object(source: &str) -> JsonObject {
    from_str(source).unwrap()
}

fn decision(source: &str) -> Json {
    from_str(source).unwrap()
}

fn provider_json(source: &str) -> ProviderChunk {
    ProviderChunk::Json(Box::new(json(source)))
}

fn content_chunk(text: &str) -> ProviderChunk {
    provider_json(&CONTENT_CHUNK.replace("TEXT", text))
}

fn content_usage_chunk() -> ProviderChunk {
    provider_json(CONTENT_USAGE_CHUNK)
}

fn finish_chunk() -> ProviderChunk {
    provider_json(FINISH_CHUNK)
}

fn usage_chunk() -> ProviderChunk {
    provider_json(USAGE_CHUNK)
}

fn empty_choice_chunk() -> ProviderChunk {
    provider_json(EMPTY_CHOICE_CHUNK)
}

fn err_context(message: &str) -> ErrorContext {
    ErrorContext {
        message: message.to_owned(),
        model: Some(MODEL.to_owned()),
        provider: Some("openai".to_owned()),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    }
}

fn bad_request_class(message: &str) -> ErrorClass {
    ErrorClass::BadRequest(Box::new(ResponseDetails::from(err_context(message))))
}

fn bad_gateway_class(message: &str) -> ErrorClass {
    ErrorClass::BadGateway(Box::new(err_context(message)))
}

fn connection_class(message: &str) -> ErrorClass {
    ErrorClass::ApiConnection(Box::new(err_context(message)))
}

fn timeout_class(message: &str) -> ErrorClass {
    ErrorClass::Timeout(Box::new(TimeoutDetails {
        context: err_context(message),
        original_status: TimeoutStatus::default(),
    }))
}

fn rate_limited_class(message: &str) -> ErrorClass {
    ErrorClass::RateLimit(Box::new(RateLimitDetails {
        context: err_context(message),
        category: RateLimitCategory::VendorRateLimit,
        dimension: None,
        headers: BTreeMap::new(),
    }))
}

fn source_error(failure: SourceFailure, class: ErrorClass, message: &str) -> StreamSourceError {
    StreamSourceError {
        failure,
        class,
        message: message.to_owned(),
    }
}

fn usage_log(prompt_tokens: i64, completion_tokens: i64) -> LogUsage {
    LogUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens: prompt_tokens.saturating_add(completion_tokens),
        prompt_tokens_details: None,
        completion_tokens_details: None,
        server_tool_use: None,
        cost: None,
        extras: IndexMap::new(),
    }
}

fn config(mode: CallMode) -> ChatStreamConfig {
    ChatStreamConfig {
        model: MODEL.to_owned(),
        provider: None,
        logging_provider: None,
        stream_options: None,
        send_stream_usage: false,
        merge_reasoning_in_choices: false,
        optional_params: None,
        hidden_params: JsonObject::new(),
        mcp_metadata: None,
        max_streaming_duration: None,
        repeated_chunk_limit: REPEATED_CHUNK_LIMIT,
        mode,
        provider_kind: ProviderKind::Builtin,
        completion_stream: None,
        logging: StreamLoggingFlags { disable: false },
    }
}

fn usage_config(mode: CallMode) -> ChatStreamConfig {
    ChatStreamConfig {
        stream_options: Some(StreamOptions {
            include_usage: true,
        }),
        send_stream_usage: true,
        ..config(mode)
    }
}

struct FakeClock {
    now: Mutex<(i64, f64)>,
}

impl FakeClock {
    fn new(seconds: i64, timestamp: f64) -> Self {
        Self {
            now: Mutex::new((seconds, timestamp)),
        }
    }

    fn set(&self, seconds: i64, timestamp: f64) {
        *self.now.lock().unwrap() = (seconds, timestamp);
    }
}

impl StreamClock for FakeClock {
    fn now(&self) -> StreamInstant {
        let (seconds, timestamp) = *self.now.lock().unwrap();
        StreamInstant { seconds, timestamp }
    }
}

struct FakeSource {
    steps: VecDeque<Result<ProviderChunk, StreamSourceError>>,
    reads: Arc<AtomicUsize>,
    closes: Arc<AtomicUsize>,
    fail_close: bool,
}

impl FakeSource {
    fn new(chunks: Vec<ProviderChunk>) -> Self {
        Self::scripted(chunks.into_iter().map(Ok).collect())
    }

    fn scripted(steps: Vec<Result<ProviderChunk, StreamSourceError>>) -> Self {
        Self {
            steps: VecDeque::from(steps),
            reads: Arc::new(AtomicUsize::new(0)),
            closes: Arc::new(AtomicUsize::new(0)),
            fail_close: false,
        }
    }

    fn failing_close() -> Self {
        let mut source = Self::scripted(Vec::new());
        source.fail_close = true;
        source
    }
}

#[async_trait]
impl ChunkSource for FakeSource {
    async fn next_chunk(&mut self) -> Result<Option<ProviderChunk>, StreamSourceError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.steps
            .pop_front()
            .map_or(Ok(None), |step| step.map(Some))
    }

    async fn close(&mut self) -> Result<(), StreamSourceError> {
        self.closes.fetch_add(1, Ordering::SeqCst);
        if self.fail_close {
            return Err(source_error(
                SourceFailure::Other,
                connection_class("close failed"),
                "close failed",
            ));
        }
        Ok(())
    }
}

#[derive(Default)]
struct Logged {
    started: Option<i64>,
    chunks: Vec<(StreamChunk, bool)>,
    completed: Vec<Vec<StreamChunk>>,
    partial: Vec<Vec<StreamChunk>>,
    failures: Vec<(ErrorClass, String)>,
    successes: Vec<(Option<Json>, bool)>,
}

struct FakeLogging {
    logged: Mutex<Logged>,
    build: Mutex<Option<StreamBuild>>,
}

impl FakeLogging {
    fn new() -> Self {
        Self {
            logged: Mutex::new(Logged::default()),
            build: Mutex::new(None),
        }
    }

    fn with_build(build: StreamBuild) -> Self {
        let logging = Self::new();
        *logging.build.lock().unwrap() = Some(build);
        logging
    }
}

impl StreamLogging for FakeLogging {
    fn started(&self, at: StreamInstant) {
        self.logged.lock().unwrap().started = Some(at.seconds);
    }

    fn log_chunk(&self, chunk: &StreamChunk, cache_hit: bool) {
        self.logged
            .lock()
            .unwrap()
            .chunks
            .push((chunk.clone(), cache_hit));
    }

    fn completed(&self, chunks: &[StreamChunk]) -> StreamBuild {
        self.logged.lock().unwrap().completed.push(chunks.to_vec());
        let build = self.build.lock().unwrap().take();
        build.unwrap_or(StreamBuild::Empty)
    }

    fn partial_usage(&self, chunks: &[StreamChunk]) {
        self.logged.lock().unwrap().partial.push(chunks.to_vec());
    }

    fn log_failure(&self, failure: &StreamFailure) {
        self.logged
            .lock()
            .unwrap()
            .failures
            .push((failure.class.clone(), failure.message.clone()));
    }

    fn log_success(&self, response: Option<Json>, cache_hit: bool) {
        self.logged
            .lock()
            .unwrap()
            .successes
            .push((response, cache_hit));
    }
}

struct FakeCache {
    stored: Mutex<Vec<Json>>,
}

impl FakeCache {
    fn new() -> Self {
        Self {
            stored: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl StreamCache for FakeCache {
    async fn store(&self, response: Json) {
        self.stored.lock().unwrap().push(response);
    }
}

struct FakeRestore {
    calls: AtomicUsize,
}

impl FakeRestore {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }
}

impl CorrelationRestore for FakeRestore {
    fn restore_correlation_context(&self, context: &mut CorrelationContext) {
        self.calls.fetch_add(1, Ordering::SeqCst);
        context.trace_id = "restored".to_owned();
        context.session_id = "restored".to_owned();
    }
}

struct FakeRules {
    calls: AtomicUsize,
    seen: Mutex<Vec<String>>,
    reject: Mutex<Option<String>>,
}

impl FakeRules {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            seen: Mutex::new(Vec::new()),
            reject: Mutex::new(None),
        }
    }

    fn reject_with(&self, message: &str) {
        *self.reject.lock().unwrap() = Some(message.to_owned());
    }
}

impl RuleEvaluator for FakeRules {
    fn evaluate(&self, _index: usize, input: &str) -> Option<RuleDecision> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.seen.lock().unwrap().push(input.to_owned());
        if self.reject.lock().unwrap().is_some() {
            return Some(RuleDecision::Boolean(false));
        }
        None
    }
}

enum HookAction {
    Keep,
    Replace(Box<StreamChunk>),
    Fail,
}

struct FakeHook {
    action: HookAction,
    seen: Mutex<Vec<StreamChunk>>,
    call_types: Mutex<Vec<Option<CallType>>>,
}

impl FakeHook {
    fn new(action: HookAction) -> Self {
        Self {
            action,
            seen: Mutex::new(Vec::new()),
            call_types: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl StreamingDeploymentHook for FakeHook {
    async fn stream_chunk(
        &self,
        _request: &RequestMap,
        call_type: Option<CallType>,
        chunk: &StreamChunk,
    ) -> Result<Option<StreamChunk>, HookError> {
        self.seen.lock().unwrap().push(chunk.clone());
        self.call_types.lock().unwrap().push(call_type);
        match &self.action {
            HookAction::Keep => Ok(None),
            HookAction::Replace(replacement) => Ok(Some(replacement.as_ref().clone())),
            HookAction::Fail => Err(HookError::Failed {
                hook: "fake".to_owned(),
                message: "boom".to_owned(),
            }),
        }
    }
}

struct World {
    clock: FakeClock,
    logging: FakeLogging,
    cache: Option<FakeCache>,
    hooks: Vec<Arc<dyn StreamingDeploymentHook>>,
    request: RequestMap,
    rules: FakeRules,
    restore: FakeRestore,
    redactor: Redactor,
}

impl World {
    fn new() -> Self {
        Self {
            clock: FakeClock::new(0, 1000.0_f64),
            logging: FakeLogging::new(),
            cache: None,
            hooks: Vec::new(),
            request: RequestMap::new(),
            rules: FakeRules::new(),
            restore: FakeRestore::new(),
            redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap(),
        }
    }

    fn with_build(build: StreamBuild) -> Self {
        let mut world = Self::new();
        world.logging = FakeLogging::with_build(build);
        world
    }

    fn cache(&self) -> Option<&dyn StreamCache> {
        let Some(cache) = &self.cache else {
            return None;
        };
        Some(cache)
    }
}

fn start<'world, S>(
    world: &'world World,
    correlation: &'world mut CorrelationContext,
    config: ChatStreamConfig,
    source: S,
) -> ChatStream<'world, S>
where
    S: ChunkSource,
{
    let ports = ChatStreamPorts {
        clock: &world.clock,
        logging: &world.logging,
        cache: world.cache(),
        hooks: &world.hooks,
        request_data: &world.request,
        call_type: "completion",
        rules: &world.rules,
        restore: &world.restore,
        correlation,
        redactor: &world.redactor,
    };
    ChatStream::new(config, source, ports)
}

async fn drain<S>(stream: &mut ChatStream<'_, S>) -> Vec<StreamChunk>
where
    S: ChunkSource,
{
    let mut seen = Vec::new();
    while let Ok(Some(chunk)) = stream.next().await {
        seen.push(chunk);
    }
    seen
}

fn only_chunk(outcome: Result<Option<StreamChunk>, ChatStreamError>) -> StreamChunk {
    let Some(chunk) = outcome.unwrap() else {
        panic!("expected a chunk");
    };
    chunk
}

fn content_of(chunk: &StreamChunk) -> String {
    chunk
        .choices
        .first()
        .and_then(|choice| choice.delta.content.clone())
        .unwrap_or_default()
}

fn finish_of(chunk: &StreamChunk) -> Option<String> {
    let choice = chunk.choices.first()?;
    choice.finish_reason.clone()
}

fn delta_fields(chunk: &StreamChunk) -> Option<&JsonObject> {
    let choice = chunk.choices.first()?;
    choice.delta.provider_specific_fields.as_ref()
}

fn hidden_usage_total(chunk: &StreamChunk) -> Option<i64> {
    let usage = chunk.hidden_params.get("usage")?;
    let JsonValue::Object(fields) = usage else {
        return None;
    };
    fields.get("total_tokens")?.as_i64()
}

fn fallback_of(error: &ChatStreamError) -> &MidStreamFallbackDetails {
    let ErrorClass::MidStreamFallback(details) = &error.class else {
        panic!("expected a mid-stream fallback");
    };
    details
}

fn captured() -> (Arc<Mutex<Vec<String>>>, impl Subscriber + Send + Sync) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(CapturedEvents(Arc::clone(&events)));
    (events, subscriber)
}

fn captured_logs(events: &Mutex<Vec<String>>) -> Vec<String> {
    events.lock().unwrap().clone()
}

fn assert_logged(logs: &[String], needle: &str) {
    assert!(
        logs.iter().any(|line| line.contains(needle)),
        "expected a log line containing {needle:?}, got {logs:?}"
    );
}

struct CapturedEvents(Arc<Mutex<Vec<String>>>);

impl<S: Subscriber> Layer<S> for CapturedEvents {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let mut fields = CapturedFields(Vec::new());
        event.record(&mut fields);
        if let Ok(mut events) = self.0.lock() {
            events.extend(fields.0);
        }
    }
}

struct CapturedFields(Vec<String>);

impl Visit for CapturedFields {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0.push(format!("{value:?}"));
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.0.push(value.to_owned());
        }
    }
}

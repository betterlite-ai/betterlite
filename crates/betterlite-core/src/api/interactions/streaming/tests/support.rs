#[derive(Debug, Error)]
enum FixtureConfigError {
    #[error("the fixture does not answer this call")]
    Unsupported,
    #[error("the fixture chunk does not decode")]
    Decode(#[source] serde_json::Error),
    #[error("the fixture chunk does not encode")]
    Encode(#[source] serde_json::Error),
    #[error(transparent)]
    Contract(#[from] InteractionsContractError),
}

struct FixtureConfig;

impl BaseInteractionsAPIConfig for FixtureConfig {
    type Error = FixtureConfigError;

    fn custom_llm_provider(&self) -> ProviderId {
        ProviderId::Gemini
    }

    fn get_supported_params(&self, _model: &str) -> Vec<String> {
        Vec::new()
    }

    fn validate_environment(
        &self,
        request: &EnvironmentRequest<'_>,
    ) -> Result<RequestHeaders, Self::Error> {
        Ok(request.headers.clone())
    }

    fn get_complete_url(&self, _request: &CompleteUrlRequest<'_>) -> Result<String, Self::Error> {
        Ok(String::new())
    }

    fn transform_request(
        &self,
        _request: &CreateInteractionRequest<'_>,
    ) -> Result<Params, Self::Error> {
        Ok(Params::new())
    }

    fn transform_response(
        &self,
        _model: Option<&str>,
        _raw_response: &InteractionsUpstream,
        _logging: &mut InteractionsLogging,
    ) -> Result<InteractionsApiResponse, Self::Error> {
        Err(FixtureConfigError::Unsupported)
    }

    fn transform_streaming_response(
        &self,
        _model: Option<&str>,
        parsed_chunk: &JsonValue,
        _logging: &mut InteractionsLogging,
    ) -> Result<InteractionsStreamChunk, Self::Error> {
        let encoded = serde_json::to_string(parsed_chunk).map_err(FixtureConfigError::Encode)?;
        let chunk = serde_json::from_str(&encoded).map_err(FixtureConfigError::Decode)?;
        Ok(chunk)
    }

    fn transform_get_request(
        &self,
        _request: &InteractionIdRequest<'_>,
    ) -> Result<PreparedRequest, Self::Error> {
        Err(FixtureConfigError::Unsupported)
    }

    fn transform_get_response(
        &self,
        _raw_response: &InteractionsUpstream,
        _logging: &mut InteractionsLogging,
    ) -> Result<InteractionsApiResponse, Self::Error> {
        Err(FixtureConfigError::Unsupported)
    }

    fn transform_delete_request(
        &self,
        _request: &InteractionIdRequest<'_>,
    ) -> Result<PreparedRequest, Self::Error> {
        Err(FixtureConfigError::Unsupported)
    }

    fn transform_delete_response(
        &self,
        _raw_response: &InteractionsUpstream,
        _logging: &mut InteractionsLogging,
        _interaction_id: &str,
    ) -> Result<DeleteInteractionResult, Self::Error> {
        Err(FixtureConfigError::Unsupported)
    }

    fn transform_cancel_request(
        &self,
        _request: &InteractionIdRequest<'_>,
    ) -> Result<PreparedRequest, Self::Error> {
        Err(FixtureConfigError::Unsupported)
    }

    fn transform_cancel_response(
        &self,
        _raw_response: &InteractionsUpstream,
        _logging: &mut InteractionsLogging,
    ) -> Result<CancelInteractionResult, Self::Error> {
        Err(FixtureConfigError::Unsupported)
    }
}

struct NoApiBase;

static NO_API_BASE: NoApiBase = NoApiBase;

impl InteractionsApiBase for NoApiBase {
    fn resolve(
        &self,
        _model: &str,
        _params: &ApiBaseParams,
    ) -> Result<Option<String>, ApiBaseError> {
        Ok(None)
    }
}

struct FailingApiBase;

impl InteractionsApiBase for FailingApiBase {
    fn resolve(
        &self,
        _model: &str,
        _params: &ApiBaseParams,
    ) -> Result<Option<String>, ApiBaseError> {
        Err(ApiBaseError::InvalidVertexLocation {
            message: String::new(),
        })
    }
}

#[derive(Debug, Error)]
#[error("the fixture stream failed")]
struct FixtureSourceError;

struct FakeAsyncResponse {
    headers: IndexMap<String, String>,
    lines: Vec<Result<String, FixtureSourceError>>,
    calls: Arc<AtomicUsize>,
}

impl FakeAsyncResponse {
    fn new(lines: &[&str]) -> Self {
        Self {
            headers: IndexMap::new(),
            lines: lines.iter().map(|line| Ok(String::from(*line))).collect(),
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }
}

struct FakeAsyncLines {
    lines: IntoIter<Result<String, FixtureSourceError>>,
}

#[async_trait]
impl InteractionsAsyncResponse for FakeAsyncResponse {
    type Error = FixtureSourceError;
    type Lines = FakeAsyncLines;

    fn headers(&self) -> IndexMap<String, String> {
        self.headers.clone()
    }

    fn aiter_lines(self) -> Self::Lines {
        self.calls.fetch_add(1, Ordering::SeqCst);
        FakeAsyncLines {
            lines: self.lines.into_iter(),
        }
    }
}

#[async_trait]
impl InteractionsLineStream for FakeAsyncLines {
    type Error = FixtureSourceError;

    async fn next_line(&mut self) -> Result<Option<String>, Self::Error> {
        match self.lines.next() {
            None => Ok(None),
            Some(Ok(line)) => Ok(Some(line)),
            Some(Err(error)) => Err(error),
        }
    }
}

struct FakeSyncResponse {
    headers: IndexMap<String, String>,
    lines: Vec<Result<String, FixtureSourceError>>,
    calls: Arc<AtomicUsize>,
}

impl FakeSyncResponse {
    fn new(lines: &[&str]) -> Self {
        Self {
            headers: IndexMap::new(),
            lines: lines.iter().map(|line| Ok(String::from(*line))).collect(),
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl InteractionsSyncResponse for FakeSyncResponse {
    type Error = FixtureSourceError;
    type Lines = IntoIter<Result<String, FixtureSourceError>>;

    fn headers(&self) -> IndexMap<String, String> {
        self.headers.clone()
    }

    fn iter_lines(self) -> Self::Lines {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.lines.into_iter()
    }
}

// every reading is one second after the one before, so construction and each dispatch get distinct instants
struct StepClock {
    reads: Mutex<i64>,
}

impl SuccessClock for StepClock {
    fn now(&self) -> DateTime<FixedOffset> {
        let mut reads = self.reads.lock().unwrap_or_else(PoisonError::into_inner);
        let now = ticked(*reads);
        *reads = reads.saturating_add(1);
        now
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DispatchRecord {
    handler: &'static str,
    response: InteractionsStreamChunk,
    start_time: DateTime<FixedOffset>,
    end_time: DateTime<FixedOffset>,
    cache_hit: Option<bool>,
    prefer_async_handlers: Option<bool>,
}

struct RecordingLogging {
    logging: InteractionsLogging,
    clock: StepClock,
    records: Mutex<Vec<DispatchRecord>>,
}

impl RecordingLogging {
    fn new(litellm_params: Params) -> Self {
        Self {
            logging: InteractionsLogging {
                model_call_details: litellm_params,
            },
            clock: StepClock {
                reads: Mutex::new(0),
            },
            records: Mutex::new(Vec::new()),
        }
    }

    fn records(&self) -> Vec<DispatchRecord> {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(
        &self,
        handler: &'static str,
        call: InteractionsSuccessCall,
        prefer_async_handlers: Option<bool>,
    ) {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(DispatchRecord {
                handler,
                response: call.response,
                start_time: call.start_time,
                end_time: call.end_time,
                cache_hit: call.cache_hit,
                prefer_async_handlers,
            });
    }
}

impl InteractionsStreamLogging for RecordingLogging {
    fn logging(&mut self) -> &mut InteractionsLogging {
        &mut self.logging
    }

    fn clock(&self) -> &dyn SuccessClock {
        &self.clock
    }

    fn dispatch_success_handlers(&self, call: InteractionsSuccessCall) {
        let prefer_async_handlers = Some(call.prefer_async_handlers);
        self.record("dispatch_success_handlers", call, prefer_async_handlers);
    }

    fn run_async_success_handler(&self, call: InteractionsSuccessCall) {
        self.record("async_success_handler", call, None);
    }

    fn submit_success_handler(&self, call: InteractionsSuccessCall) {
        self.record("success_handler", call, None);
    }
}

struct NoHooks;

impl ApiBaseHooks for NoHooks {
    fn login_free_api_base(&self, _provider: LoginFreeProvider) -> Option<String> {
        None
    }

    fn vertex_claude_url(
        &self,
        _location: &str,
        _project: &str,
        _model: &str,
        _stream: bool,
    ) -> Result<String, ApiBaseError> {
        Err(ApiBaseError::InvalidVertexLocation {
            message: String::new(),
        })
    }
}

fn moment() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-05-20T00:00:00+00:00").expect("the fixture time parses")
}

fn ticked(seconds: i64) -> DateTime<FixedOffset> {
    let step = Seconds::try_seconds(seconds).expect("the fixture step is small");
    moment()
        .checked_add_signed(step)
        .expect("the fixture time stays in range")
}

fn litellm_params(api_base: &str) -> Params {
    let fields = IndexMap::from([(
        String::from("api_base"),
        JsonValue::Text(api_base.to_owned()),
    )]);
    Params::from([(String::from("litellm_params"), JsonValue::Object(fields))])
}

fn metadata(model_id: &str) -> Params {
    let info = IndexMap::from([(String::from("id"), JsonValue::Text(model_id.to_owned()))]);
    Params::from([(String::from("model_info"), JsonValue::Object(info))])
}

type TestAsyncIterator<'base> =
    InteractionsStreamIterator<'base, FakeAsyncResponse, FixtureConfig, RecordingLogging>;
type TestSyncIterator<'base> =
    SyncInteractionsStreamIterator<'base, FakeSyncResponse, FixtureConfig, RecordingLogging>;

fn setup<'base>(
    logging: &'base mut RecordingLogging,
    config: &'base FixtureConfig,
    api_base: &'base dyn InteractionsApiBase,
) -> InteractionsStreamSetup<'base, FixtureConfig, RecordingLogging> {
    InteractionsStreamSetup {
        model: Some("gemini-2.5-flash"),
        config,
        logging,
        litellm_metadata: None,
        custom_llm_provider: None,
        api_base,
    }
}

async fn drain_async(iterator: &mut TestAsyncIterator<'_>) -> Vec<InteractionsStreamChunk> {
    let mut events = Vec::new();
    while let Some(item) = iterator.next_chunk().await {
        events.push(item.expect("the fixture stream yields a chunk"));
    }
    events
}

fn drain_sync(iterator: &mut TestSyncIterator<'_>) -> Vec<InteractionsStreamChunk> {
    let mut events = Vec::new();
    while let Some(item) = iterator.next_chunk() {
        events.push(item.expect("the fixture stream yields a chunk"));
    }
    events
}

fn ids(events: &[InteractionsStreamChunk]) -> Vec<&str> {
    events
        .iter()
        .map(|event| event.id.as_deref().unwrap_or_default())
        .collect()
}

fn resolution_context<'context>(
    catalog: &'context ModelCatalog,
    known: &'context BTreeSet<String>,
    custom: &'context BTreeSet<String>,
    environment: &'context BTreeMap<String, String>,
) -> ResolutionContext<'context> {
    ResolutionContext {
        catalog,
        known_model_names: known,
        custom_providers: custom,
        environment,
        registry: JSONProviderRegistry::load(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    }
}

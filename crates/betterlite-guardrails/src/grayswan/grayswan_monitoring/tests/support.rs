fn object(entries: Vec<(&str, GuardrailValue)>) -> GuardrailValue {
    GuardrailValue::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

fn text(value: &str) -> GuardrailValue {
    GuardrailValue::Text(value.to_owned())
}

fn lookup(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

fn settings() -> Configured {
    configured("http://example", OnFlaggedAction::Monitor, true)
}

fn blocked_api_error() -> GraySwanGuardrailAPIError {
    GraySwanGuardrailAPIError::from_failure(GraySwanFailure::Blocked {
        reason: "blocked".to_owned(),
        violation_location: "output",
        violation: 1.0,
        violated_rules: Vec::new(),
        mutation: None,
        ipi: None,
    })
}

fn log_target(headers: Vec<(&str, GuardrailValue)>) -> GuardrailLogTarget {
    GuardrailLogTarget {
        litellm_params: None,
        model_call_params: Some(BTreeMap::from([(
            "metadata".to_owned(),
            object(vec![("headers", object(headers))]),
        )])),
    }
}

fn request_with<'call>(
    messages: Vec<GuardrailValue>,
    dynamic: &'call GraySwanDynamicParams,
    request_data: &'call Object,
) -> PreparePayloadRequest<'call> {
    PreparePayloadRequest {
        messages,
        tools: None,
        dynamic,
        request_data,
        log_target: None,
    }
}

fn nested(depth: usize) -> GuardrailValue {
    let mut value = text("leaf");
    for _ in 0..depth {
        value = object(vec![("child", value)]);
    }
    value
}

fn descend(mut value: &GuardrailValue, mut steps: usize) -> &GuardrailValue {
    while steps > 0 {
        let GuardrailValue::Object(fields) = value else {
            panic!("not an object");
        };
        value = fields.get("child").expect("child");
        steps -= 1;
    }
    value
}

fn routed_data(route: &str) -> Object {
    Object::from([(
        "litellm_metadata".to_owned(),
        object(vec![("user_api_key_request_route", text(route))]),
    )])
}

fn route_catalog() -> RouteCatalog {
    RouteCatalog {
        source: "test".to_owned(),
        call_types: vec![CallType::Completion, CallType::Embedding],
        routes: vec![RouteEntry {
            route: "/v1/chat/completions".to_owned(),
            call_types: vec![CallType::Completion],
        }],
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the fake translation never fails")]
struct FakeFailure;

struct FakeTranslation {
    messages: Option<Vec<GuardrailMessage>>,
}

impl BaseTranslation for FakeTranslation {
    type Guardrail = ();
    type LogTarget = ();
    type Failure = FakeFailure;

    fn process_input_messages(
        &self,
        _data: &mut BTreeMap<String, GuardrailValue>,
        _guardrail: &(),
        _logging_obj: Option<&mut ()>,
    ) -> impl Future<Output = Result<(), Self::Failure>> + Send {
        ready(Ok(()))
    }

    fn process_output_response(
        &self,
        response: GuardrailValue,
        _guardrail: &(),
        _call: TranslationCall<'_, ()>,
    ) -> impl Future<Output = Result<GuardrailValue, Self::Failure>> + Send {
        ready(Ok(response))
    }

    fn get_structured_messages(
        &self,
        _data: &BTreeMap<String, GuardrailValue>,
    ) -> Option<Vec<GuardrailMessage>> {
        self.messages.clone()
    }
}

fn registry_for<'source>(
    translation: &'source FakeTranslation,
    call_type: CallType,
) -> TranslationRegistry<&'source dyn StructuredMessageSource> {
    let source: &dyn StructuredMessageSource = translation;
    TranslationRegistry {
        entries: vec![TranslationEntry {
            call_type,
            translation: source,
            chat_completions: false,
        }],
    }
}

fn run_context<'call>(
    request_data: &'call Object,
    logging: Option<&'call str>,
    scope: ScopeFlags,
    translations: &'call TranslationRegistry<&'call dyn StructuredMessageSource>,
    routes: &'call RouteCatalog,
) -> (Vec<GuardrailValue>, Option<Vec<GuardrailValue>>) {
    post_call_context(PostCallContextRequest {
        request_data,
        logging_call_type: logging,
        scope,
        translations,
        routes,
    })
}

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<CapturedRequest>>);

#[derive(Default)]
struct CapturedRequest {
    method: Option<String>,
    path: Option<String>,
    authorization: Option<String>,
    api_key: Option<String>,
    content_type: Option<String>,
    body: Vec<u8>,
}

impl Capture {
    fn record(&self, method: &Method, uri: &Uri, headers: &HeaderMap, body: &Bytes) {
        let mut captured = self.0.lock().unwrap();
        captured.method = Some(method.as_str().to_owned());
        captured.path = Some(uri.path().to_owned());
        captured.authorization = header_text(headers, AUTHORIZATION.as_str());
        captured.api_key = header_text(headers, "grayswan-api-key");
        captured.content_type = header_text(headers, CONTENT_TYPE.as_str());
        captured.body = body.to_vec();
    }
}

fn header_text(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned)
}

#[derive(Clone)]
struct Answer {
    status: AxumStatus,
    body: String,
    delay: Option<Duration>,
}

fn answer(status: AxumStatus, body: &str) -> Answer {
    Answer {
        status,
        body: body.to_owned(),
        delay: None,
    }
}

async fn answering(
    State((capture, reply)): State<(Capture, Answer)>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> (AxumStatus, String) {
    if let Some(delay) = reply.delay {
        tokio::time::sleep(delay).await;
    }
    capture.record(&method, &uri, &headers, &body);
    (reply.status, reply.body.clone())
}

async fn serve(router: Router) -> (String, JoinSet<Result<(), io::Error>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    (base, server)
}

async fn capture_server(reply: Answer) -> (Capture, String, JoinSet<Result<(), io::Error>>) {
    let capture = Capture::default();
    let router = Router::new()
        .route("/cygnal/monitor", post(answering))
        .with_state((capture.clone(), reply));
    let (base, server) = serve(router).await;
    (capture, base, server)
}

#[derive(Deserialize)]
struct CapturedPayload {
    messages: Vec<GuardrailMessage>,
    #[serde(default)]
    tools: Option<Vec<GuardrailValue>>,
    #[serde(default)]
    litellm_metadata: Option<GuardrailValue>,
}

fn captured_payload(capture: &Capture) -> CapturedPayload {
    let captured = capture.0.lock().unwrap();
    serde_json::from_slice(&captured.body).expect("captured payload json")
}

fn payload() -> GraySwanPayload {
    GraySwanPayload {
        messages: vec![plain_message("user", "hi")],
        tools: None,
        categories: None,
        policy_id: None,
        reasoning_mode: None,
        metadata: None,
        litellm_metadata: None,
    }
}

#[derive(Clone)]
struct StreamPlan(Arc<Vec<Result<Bytes, String>>>);

async fn streaming(State(StreamPlan(items)): State<StreamPlan>) -> (AxumStatus, Body) {
    let chunks: Vec<Result<Bytes, io::Error>> = items
        .iter()
        .map(|item| match item {
            Ok(bytes) => Ok(bytes.clone()),
            Err(error) => Err(io::Error::other(error.clone())),
        })
        .collect();
    (
        AxumStatus::OK,
        Body::from_stream(futures_util::stream::iter(chunks)),
    )
}

async fn call_streaming(
    chunks: Vec<Result<Bytes, String>>,
) -> Result<GraySwanResponse, GraySwanGuardrailAPIError> {
    let router = Router::new()
        .route("/cygnal/monitor", post(streaming))
        .with_state(StreamPlan(Arc::new(chunks)));
    let (base, _server) = serve(router).await;
    let settings = configured(&base, OnFlaggedAction::Monitor, true);
    call_grayswan_api(&reqwest::Client::default(), &settings, &payload()).await
}

fn records(data: &Object) -> &[GuardrailValue] {
    let Some(GuardrailValue::Object(metadata)) = data.get("metadata") else {
        return &[];
    };
    match metadata.get("standard_logging_guardrail_information") {
        Some(GuardrailValue::Array(entries)) => entries,
        _ => &[],
    }
}

fn log_failure(data: &mut Object, status_code: Option<u16>) {
    log_guardrail_failure(
        &super::base(),
        GuardrailFailureLog {
            message: "boom".to_owned(),
            status_code,
            started_at: SystemTime::UNIX_EPOCH,
            finished_at: SystemTime::UNIX_EPOCH,
            request_data: data,
            log_target: None,
        },
    );
}

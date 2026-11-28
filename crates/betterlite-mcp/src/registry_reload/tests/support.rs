const PLAIN_ROW: &str = r#"{"server_id":"srv-1","transport":"http","url":"https://mcp.test"}"#;
const OAUTH_ROW: &str =
    r#"{"server_id":"srv-1","transport":"http","url":"https://mcp.test","auth_type":"oauth2"}"#;

struct FakeDecryptor;

impl Decryptor for FakeDecryptor {
    fn decrypt(&self, value: &str, _key: &str) -> String {
        value.to_owned()
    }
}

#[derive(Default)]
struct FakeStdioPolicy {
    blocked: bool,
}

impl StdioPolicy for FakeStdioPolicy {
    fn is_blocked(&self) -> bool {
        self.blocked
    }
}

#[derive(Default)]
struct GateSlots(Mutex<Vec<(String, bool)>>);

impl GateSlots {
    fn entries(&self) -> Vec<(String, bool)> {
        self.0.lock().unwrap().clone()
    }
}

impl DeferredSlots for GateSlots {
    fn record_deferred(&self, server_id: &str, discovery_deferred: bool) {
        self.0
            .lock()
            .unwrap()
            .push((server_id.to_owned(), discovery_deferred));
    }
}

#[derive(Default)]
struct FetcherState {
    routes: BTreeMap<String, (u16, String)>,
    calls: Vec<String>,
}

#[derive(Clone, Default)]
struct FakeFetcher {
    state: Arc<Mutex<FetcherState>>,
}

impl FakeFetcher {
    fn route(&self, url: &str, status: u16, body: &str) {
        self.state
            .lock()
            .unwrap()
            .routes
            .insert(url.to_owned(), (status, body.to_owned()));
    }

    fn calls(&self) -> Vec<String> {
        self.state.lock().unwrap().calls.clone()
    }
}

impl MetadataFetcher for FakeFetcher {
    fn get_json(
        &self,
        url: &str,
        _headers: &[(String, String)],
        _timeout: Duration,
    ) -> impl Future<Output = Result<FetchedDocument, FetchError>> + Send {
        let (status, body) = {
            let mut state = self.state.lock().unwrap();
            state.calls.push(url.to_owned());
            state
                .routes
                .get(url)
                .cloned()
                .unwrap_or((404_u16, String::new()))
        };
        ready(Ok(FetchedDocument {
            status: StatusCode::from_u16(status).unwrap_or(StatusCode::NOT_FOUND),
            headers: HeaderMap::new(),
            body: body.into_bytes(),
        }))
    }
}

#[derive(Default)]
struct RowsState {
    rows: Vec<McpServerTable>,
    failure: Option<String>,
}

#[derive(Default)]
struct FakeRows {
    state: Mutex<RowsState>,
}

impl FakeRows {
    fn set(&self, rows: Vec<McpServerTable>) {
        self.state.lock().unwrap().rows = rows;
    }

    fn fail_with(&self, server_id: &str) {
        self.state.lock().unwrap().failure = Some(server_id.to_owned());
    }
}

impl ServerRows for FakeRows {
    fn find_active(
        &self,
    ) -> impl Future<Output = Result<Vec<McpServerTable>, McpTableError>> + Send {
        let state = self.state.lock().unwrap();
        let failure = state.failure.clone();
        let rows = state.rows.clone();
        drop(state);
        ready(failure.map_or(Ok(rows), |server_id| {
            Err(McpTableError::ServerNotFound { server_id })
        }))
    }
}

#[derive(Default)]
struct SinkState {
    tools: BTreeSet<String>,
    owners: BTreeMap<String, String>,
    rebuilds: usize,
    unregistered: Vec<String>,
}

#[derive(Default)]
struct FakeSink {
    state: Mutex<SinkState>,
}

impl FakeSink {
    fn seed_tools(&self, names: &[&str]) {
        let mut state = self.state.lock().unwrap();
        for name in names {
            state.tools.insert((*name).to_owned());
        }
    }

    fn tools(&self) -> BTreeSet<String> {
        self.state.lock().unwrap().tools.clone()
    }

    fn owners(&self) -> BTreeMap<String, String> {
        self.state.lock().unwrap().owners.clone()
    }

    fn rebuilds(&self) -> usize {
        self.state.lock().unwrap().rebuilds
    }

    fn unregistered(&self) -> Vec<String> {
        self.state.lock().unwrap().unregistered.clone()
    }
}

impl OpenApiToolSink for FakeSink {
    fn register_tool(
        &self,
        name: &str,
        _description: &str,
        _input_schema: Object,
        _config: OpenApiToolConfig,
    ) {
        self.state.lock().unwrap().tools.insert(name.to_owned());
    }

    fn set_tool_owner(&self, tool_name: &str, server_prefix: &str) {
        self.state
            .lock()
            .unwrap()
            .owners
            .insert(tool_name.to_owned(), server_prefix.to_owned());
    }

    fn rebuild_tool_name_mapping(&self) {
        let mut state = self.state.lock().unwrap();
        state.rebuilds = state.rebuilds.saturating_add(1);
    }
}

impl ToolRegistrySink for FakeSink {
    fn unregister_prefix(&self, prefix: &str) -> usize {
        let mut state = self.state.lock().unwrap();
        state.unregistered.push(prefix.to_owned());
        let doomed: Vec<String> = state
            .tools
            .iter()
            .filter(|name| name.starts_with(prefix))
            .cloned()
            .collect();
        for name in &doomed {
            state.tools.remove(name);
        }
        doomed.len()
    }
}

#[derive(Default)]
struct FakeInstructions {
    clears: AtomicUsize,
}

impl FakeInstructions {
    fn clears(&self) -> usize {
        self.clears.load(Ordering::SeqCst)
    }
}

impl InstructionsCache for FakeInstructions {
    fn clear(&self) {
        self.clears.fetch_add(1, Ordering::SeqCst);
    }
}

#[derive(Default)]
struct FakeCaches {
    invalidated: Mutex<Vec<String>>,
}

impl FakeCaches {
    fn invalidated(&self) -> Vec<String> {
        self.invalidated.lock().unwrap().clone()
    }
}

impl CacheInvalidator for FakeCaches {
    fn invalidate_server(&self, server_id: &str) {
        self.invalidated.lock().unwrap().push(server_id.to_owned());
    }
}

#[derive(Default)]
struct HydratorState {
    result: bool,
    evict_from: Option<SharedRegistry>,
    calls: Vec<String>,
}

#[derive(Default)]
struct FakeHydrator {
    state: Mutex<HydratorState>,
}

impl FakeHydrator {
    fn returning(result: bool) -> Self {
        Self {
            state: Mutex::new(HydratorState {
                result,
                evict_from: None,
                calls: Vec::new(),
            }),
        }
    }

    fn evicting(self, registry: &SharedRegistry) -> Self {
        self.state.lock().unwrap().evict_from = Some(Arc::clone(registry));
        self
    }

    fn calls(&self) -> Vec<String> {
        self.state.lock().unwrap().calls.clone()
    }
}

impl DcrHydrator for FakeHydrator {
    fn hydrate(&self, server: &mut MCPServer) -> impl Future<Output = bool> + Send {
        let mut state = self.state.lock().unwrap();
        state.calls.push(server.fields.server_id.clone());
        let result = state.result;
        let evict_from = state.evict_from.clone();
        drop(state);
        if let Some(registry) = evict_from {
            let mut held = registry.try_write().unwrap();
            held.config_servers.shift_remove(&server.fields.server_id);
        }
        if result {
            server.fields.instructions = Some(String::from("hydrated"));
        }
        ready(result)
    }
}

#[derive(Clone)]
enum ToolPlan {
    Tools(Vec<String>),
    UpstreamAuth,
    Failed,
}

#[derive(Default)]
struct ListerState {
    plans: BTreeMap<String, ToolPlan>,
    calls: Vec<String>,
}

#[derive(Default)]
struct FakeLister {
    state: Mutex<ListerState>,
}

impl FakeLister {
    fn plan(&self, server_id: &str, plan: ToolPlan) {
        self.state
            .lock()
            .unwrap()
            .plans
            .insert(server_id.to_owned(), plan);
    }

    fn calls(&self) -> Vec<String> {
        self.state.lock().unwrap().calls.clone()
    }
}

impl ToolLister for FakeLister {
    fn list_tools(
        &self,
        server: &MCPServer,
    ) -> impl Future<Output = Result<Vec<McpTool>, ToolListError>> + Send {
        let plan = {
            let mut state = self.state.lock().unwrap();
            state.calls.push(server.fields.server_id.clone());
            state
                .plans
                .get(&server.fields.server_id)
                .cloned()
                .unwrap_or(ToolPlan::Tools(Vec::new()))
        };
        ready(match plan {
            ToolPlan::Tools(names) => Ok(listed_tools(names)),
            ToolPlan::UpstreamAuth => Err(ToolListError::UpstreamAuth(UpstreamAuthError {
                status_code: 401,
                www_authenticate: None,
                server_name: server.fields.name.clone(),
            })),
            ToolPlan::Failed => Err(ToolListError::Failed(String::from("boom"))),
        })
    }
}

fn listed_tools(names: Vec<String>) -> Vec<McpTool> {
    names
        .into_iter()
        .map(|name| McpTool {
            name,
            description: None,
            input_schema: Some(Object::new()),
            extra: Object::new(),
        })
        .collect()
}

type TestMutator<'mutator> = RegistryMutator<
    'mutator,
    FakeDecryptor,
    FakeFetcher,
    FakeStdioPolicy,
    GateSlots,
    TokioClock,
    FakeRows,
    FakeHydrator,
    FakeLister,
    FakeSink,
    FakeInstructions,
    FakeCaches,
    SystemResolver,
>;

struct Fixture {
    decryptor: FakeDecryptor,
    cipher: DbCipher,
    fetcher: FakeFetcher,
    settings: DiscoverySettings,
    stdio: FakeStdioPolicy,
    gate_slots: GateSlots,
    prefixes: PrefixPolicy,
    registry: SharedRegistry,
    rows: FakeRows,
    tools: FakeSink,
    owners: SharedToolOwners,
    instructions: FakeInstructions,
    caches: FakeCaches,
    hydrator: FakeHydrator,
    lister: Arc<FakeLister>,
    tasks: TaskTracker,
    spec_fetcher: UserUrlFetcher<SystemResolver>,
    client: ProxiedClient,
    discovery: Arc<DiscoverySlots<FakeFetcher, TokioClock>>,
}

fn test_fixture() -> Fixture {
    let registry: SharedRegistry = Arc::new(AsyncRwLock::new(ServerRegistry::new(
        RegistrySettings {
            public_mcp_servers: Vec::new(),
            public_hub_strict: false,
            internal_ip_ranges: None,
        },
        Vec::new(),
        Vec::new(),
    )));
    let fetcher = FakeFetcher::default();
    let settings = DiscoverySettings::default();
    let discovery = Arc::new(DiscoverySlots::new(
        Arc::clone(&registry),
        fetcher.clone(),
        TokioClock,
        settings.clone(),
        TaskTracker::new(),
    ));
    Fixture {
        decryptor: FakeDecryptor,
        cipher: test_cipher(),
        fetcher,
        settings,
        stdio: FakeStdioPolicy::default(),
        gate_slots: GateSlots::default(),
        prefixes: PrefixPolicy::default(),
        registry,
        rows: FakeRows::default(),
        tools: FakeSink::default(),
        owners: Arc::new(StdRwLock::new(ToolOwners::default())),
        instructions: FakeInstructions::default(),
        caches: FakeCaches::default(),
        hydrator: FakeHydrator::default(),
        lister: Arc::new(FakeLister::default()),
        tasks: TaskTracker::new(),
        spec_fetcher: test_fetcher(),
        client: test_client(),
        discovery,
    }
}

impl Fixture {
    fn builder(&self) -> ServerBuilder<'_, FakeDecryptor, FakeFetcher, FakeStdioPolicy, GateSlots> {
        ServerBuilder {
            decryptor: &self.decryptor,
            env_cipher: &self.cipher,
            fetcher: &self.fetcher,
            settings: &self.settings,
            stdio: &self.stdio,
            slots: &self.gate_slots,
            prefixes: self.prefixes.clone(),
        }
    }

    fn mutator<'mutator>(
        &'mutator self,
        builder: &'mutator ServerBuilder<
            'mutator,
            FakeDecryptor,
            FakeFetcher,
            FakeStdioPolicy,
            GateSlots,
        >,
    ) -> TestMutator<'mutator> {
        RegistryMutator {
            builder,
            registry: Arc::clone(&self.registry),
            slots: Arc::clone(&self.discovery),
            rows: &self.rows,
            tools: &self.tools,
            owners: Arc::clone(&self.owners),
            instructions: &self.instructions,
            caches: &self.caches,
            dcr: &self.hydrator,
            lister: Arc::clone(&self.lister),
            tasks: self.tasks.clone(),
            spec_fetcher: &self.spec_fetcher,
            client: &self.client,
            warned_shadowed: WarnedIds::default(),
            warned_capturing: WarnedIds::default(),
        }
    }

    async fn put_db_server(&self, server: MCPServer) {
        let server_id = server.fields.server_id.clone();
        self.put_db_entry(&server_id, server).await;
    }

    async fn put_db_entry(&self, key: &str, server: MCPServer) {
        self.registry
            .write()
            .await
            .db_servers
            .insert(key.to_owned(), server);
    }

    async fn put_config_server(&self, server: MCPServer) {
        let server_id = server.fields.server_id.clone();
        self.registry
            .write()
            .await
            .config_servers
            .insert(server_id, server);
    }

    async fn db_server(&self, server_id: &str) -> Option<MCPServer> {
        self.registry
            .read()
            .await
            .db_servers
            .get(server_id)
            .cloned()
    }

    async fn db_ids(&self) -> Vec<String> {
        self.registry
            .read()
            .await
            .db_servers
            .keys()
            .cloned()
            .collect()
    }

    async fn config_server(&self, server_id: &str) -> Option<MCPServer> {
        self.registry
            .read()
            .await
            .config_servers
            .get(server_id)
            .cloned()
    }

    async fn settle(&self) {
        self.tasks.close();
        self.tasks.wait().await;
    }
}

fn test_cipher() -> DbCipher {
    StoredCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from("test-salt")),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

fn test_fetcher() -> UserUrlFetcher<SystemResolver> {
    UserUrlFetcher {
        validator: UrlValidator::new(SystemResolver, &[], &Verification::Enabled),
        validation: false,
    }
}

fn test_client() -> ProxiedClient {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    ProxiedClient::build(base_builder(), &policy, TransportLimits::DEFAULT).unwrap()
}

fn row(source: &str) -> McpServerTable {
    serde_json::from_str(source).unwrap()
}

fn server(source: &str) -> MCPServer {
    serde_json::from_str(source).unwrap()
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn encrypt(cipher: &DbCipher, plaintext: &str) -> String {
    let sealed = cipher
        .encrypt_value(StoredValue::<()>::Text(SecretString::from(plaintext)), None)
        .unwrap();
    match sealed {
        StoredValue::Text(text) => text.expose_secret().to_owned(),
        StoredValue::Other(()) => String::new(),
    }
}

fn env_value(server: &MCPServer, name: &str) -> Option<JsonValue> {
    let wanted_name = JsonValue::String(name.to_owned());
    let entries = server.fields.env_vars.as_ref()?;
    let entry = entries
        .iter()
        .find(|entry| entry.get("name") == Some(&wanted_name))?;
    entry.get("value").cloned()
}

fn corpus(name: &str) -> String {
    format!(
        "{}/fuzz/corpus/openapi_spec/{name}",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn colliding_ids() -> (String, String) {
    let mut seen: BTreeMap<String, String> = BTreeMap::new();
    for index in 0..10_000_u32 {
        let server_id = format!("collide-{index}");
        let prefix = compute_short_server_prefix(&server_id, 0).unwrap();
        if let Some(previous) = seen.insert(prefix, server_id.clone()) {
            return (previous, server_id);
        }
    }
    panic!("no short-prefix collision found");
}

fn occurrences(haystack: &str, needle: &str) -> usize {
    haystack.matches(needle).count()
}

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let Ok(mut captured) = self.0.lock() else {
            return Err(io::Error::other("the capture buffer lock is poisoned"));
        };
        captured.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn capture() -> (Captured, DefaultGuard) {
    capture_at(Level::DEBUG)
}

fn capture_at(level: Level) -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_max_level(level)
        .with_ansi(false)
        .finish();
    (captured, set_default(subscriber))
}

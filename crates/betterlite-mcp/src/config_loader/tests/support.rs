#[derive(Default)]
struct FakeDecryptor;

impl Decryptor for FakeDecryptor {
    fn decrypt(&self, value: &str, _key: &str) -> String {
        value.to_owned()
    }
}

#[derive(Clone, Copy, Default)]
struct FakeStdioPolicy(bool);

impl StdioPolicy for FakeStdioPolicy {
    fn is_blocked(&self) -> bool {
        self.0
    }
}

#[derive(Default)]
struct FakeSlots;

impl DeferredSlots for FakeSlots {
    fn record_deferred(&self, _server_id: &str, _discovery_deferred: bool) {}
}

#[derive(Default)]
struct FakeState {
    routes: BTreeMap<String, (u16, String)>,
    calls: Vec<String>,
}

#[derive(Clone, Default)]
struct FakeFetcher {
    state: Arc<Mutex<FakeState>>,
}

impl FakeFetcher {
    fn route(&self, url: &str, status: u16, body: &str) {
        self.state
            .lock()
            .unwrap()
            .routes
            .insert(url.to_owned(), (status, body.to_owned()));
    }

    fn with_metadata() -> Self {
        let fetcher = Self::default();
        fetcher.route("https://mcp.test", 200, "{}");
        fetcher.route(
            "https://mcp.test/.well-known/oauth-protected-resource",
            200,
            "{\"authorization_servers\":[\"https://as.test\"]}",
        );
        fetcher.route(
            "https://as.test/.well-known/oauth-authorization-server",
            200,
            "{\"issuer\":\"https://as.test\",\"authorization_endpoint\":\"https://as.test/authorize\",\"token_endpoint\":\"https://as.test/token\",\"registration_endpoint\":\"https://as.test/register\",\"scopes_supported\":[\"read\",\"write\"]}",
        );
        fetcher
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
        let document = FetchedDocument {
            status: StatusCode::from_u16(status).unwrap_or(StatusCode::NOT_FOUND),
            headers: HeaderMap::new(),
            body: body.into_bytes(),
        };
        ready(Ok(document))
    }
}

#[derive(Default)]
struct FakeDcrHydrator {
    result: AtomicBool,
    hydrated: Mutex<Vec<String>>,
}

impl FakeDcrHydrator {
    fn returning(result: bool) -> Self {
        Self {
            result: AtomicBool::new(result),
            hydrated: Mutex::new(Vec::new()),
        }
    }

    fn hydrated(&self) -> Vec<String> {
        self.hydrated.lock().unwrap().clone()
    }
}

impl DcrHydrator for FakeDcrHydrator {
    fn hydrate(&self, server: &mut MCPServer) -> impl Future<Output = bool> + Send {
        self.hydrated
            .lock()
            .unwrap()
            .push(server.fields.server_id.clone());
        let result = self.result.load(Ordering::SeqCst);
        if result {
            server.fields.instructions = Some(String::from("hydrated"));
        }
        ready(result)
    }
}

struct FakeClientStore {
    credentials: Option<JsonValue>,
}

impl DcrClientStore for FakeClientStore {
    type Error = Infallible;

    fn client_credentials(
        &self,
        _server_id: &str,
    ) -> impl Future<Output = Result<Option<JsonValue>, StoreReadError<Self::Error>>> + Send {
        ready(Ok(self.credentials.clone()))
    }
}

struct RegisteredTool {
    name: String,
    description: String,
    config: OpenApiToolConfig,
}

#[derive(Default)]
struct SinkState {
    tools: Vec<RegisteredTool>,
    owners: BTreeMap<String, String>,
    rebuilds: usize,
}

#[derive(Default)]
struct FakeToolSink {
    state: Mutex<SinkState>,
}

impl FakeToolSink {
    fn names(&self) -> Vec<String> {
        self.state
            .lock()
            .unwrap()
            .tools
            .iter()
            .map(|tool| tool.name.clone())
            .collect()
    }

    fn descriptions(&self) -> BTreeMap<String, String> {
        self.state
            .lock()
            .unwrap()
            .tools
            .iter()
            .map(|tool| (tool.name.clone(), tool.description.clone()))
            .collect()
    }

    fn config_for(&self, name: &str) -> OpenApiToolConfig {
        let state = self.state.lock().unwrap();
        let config = state
            .tools
            .iter()
            .find(|tool| tool.name == name)
            .unwrap()
            .config
            .clone();
        drop(state);
        config
    }

    fn owners(&self) -> BTreeMap<String, String> {
        self.state.lock().unwrap().owners.clone()
    }

    fn rebuilds(&self) -> usize {
        self.state.lock().unwrap().rebuilds
    }
}

impl OpenApiToolSink for FakeToolSink {
    fn register_tool(
        &self,
        name: &str,
        description: &str,
        _input_schema: Object,
        config: OpenApiToolConfig,
    ) {
        self.state.lock().unwrap().tools.push(RegisteredTool {
            name: name.to_owned(),
            description: description.to_owned(),
            config,
        });
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

impl ServerDefinitionCaches for FakeCaches {
    fn invalidate(&self, server_id: &str) {
        self.invalidated.lock().unwrap().push(server_id.to_owned());
    }
}

struct Fixture {
    decryptor: FakeDecryptor,
    cipher: DbCipher,
    fetcher: FakeFetcher,
    settings: DiscoverySettings,
    stdio: FakeStdioPolicy,
    gate_slots: FakeSlots,
    prefixes: PrefixPolicy,
    registry: SharedRegistry,
    dcr: FakeDcrHydrator,
    tools: FakeToolSink,
    instructions: FakeInstructions,
    caches: FakeCaches,
    spec_fetcher: UserUrlFetcher<SystemResolver>,
    client: ProxiedClient,
    discovery: Arc<DiscoverySlots<FakeFetcher, TokioClock>>,
    id_jag_gap: Option<String>,
}

impl Fixture {
    fn new() -> Self {
        Self::with_startup(false)
    }

    fn with_startup(on_startup: bool) -> Self {
        let settings = settings_with_startup(on_startup);
        let registry: SharedRegistry = Arc::new(RwLock::new(ServerRegistry::new(
            RegistrySettings {
                public_mcp_servers: Vec::new(),
                public_hub_strict: false,
                internal_ip_ranges: None,
            },
            Vec::new(),
            Vec::new(),
        )));
        let fetcher = FakeFetcher::default();
        let discovery = Arc::new(DiscoverySlots::new(
            Arc::clone(&registry),
            fetcher.clone(),
            TokioClock,
            settings.clone(),
            TaskTracker::new(),
        ));
        Self {
            decryptor: FakeDecryptor,
            cipher: test_cipher(),
            fetcher,
            settings,
            stdio: FakeStdioPolicy::default(),
            gate_slots: FakeSlots,
            prefixes: PrefixPolicy::default(),
            registry,
            dcr: FakeDcrHydrator::default(),
            tools: FakeToolSink::default(),
            instructions: FakeInstructions::default(),
            caches: FakeCaches::default(),
            spec_fetcher: test_fetcher(),
            client: test_client(),
            discovery,
            id_jag_gap: None,
        }
    }

    fn with_metadata() -> Self {
        let mut fixture = Self::with_startup(true);
        fixture.mount_metadata();
        fixture
    }

    fn mount_metadata(&mut self) {
        self.fetcher = FakeFetcher::with_metadata();
        self.discovery = Arc::new(DiscoverySlots::new(
            Arc::clone(&self.registry),
            self.fetcher.clone(),
            TokioClock,
            self.settings.clone(),
            TaskTracker::new(),
        ));
    }

    fn builder(&self) -> ServerBuilder<'_, FakeDecryptor, FakeFetcher, FakeStdioPolicy, FakeSlots> {
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

    fn loader<'load>(
        &'load self,
        builder: &'load ServerBuilder<
            'load,
            FakeDecryptor,
            FakeFetcher,
            FakeStdioPolicy,
            FakeSlots,
        >,
    ) -> ConfigLoader<
        'load,
        FakeDecryptor,
        FakeFetcher,
        FakeStdioPolicy,
        FakeSlots,
        TokioClock,
        FakeDcrHydrator,
        FakeToolSink,
        FakeInstructions,
        FakeCaches,
        SystemResolver,
    > {
        ConfigLoader {
            builder,
            slots: &self.discovery,
            registry: Arc::clone(&self.registry),
            dcr: &self.dcr,
            tools: &self.tools,
            instructions: &self.instructions,
            caches: &self.caches,
            spec_fetcher: &self.spec_fetcher,
            client: &self.client,
            id_jag_gap: self.id_jag_gap.as_deref(),
        }
    }

    async fn load(&self, config: &IndexMap<String, McpServerConfig>) -> Result<(), ConfigError> {
        self.load_with(config, None).await
    }

    async fn load_with(
        &self,
        config: &IndexMap<String, McpServerConfig>,
        mcp_aliases: Option<&IndexMap<String, String>>,
    ) -> Result<(), ConfigError> {
        self.loader(&self.builder()).load(config, mcp_aliases).await
    }

    async fn maybe_register(&self, server: &MCPServer) -> Result<(), ConfigError> {
        self.loader(&self.builder())
            .maybe_register_openapi_tools(server)
            .await
    }

    async fn add_db_server(&self, server: MCPServer) {
        self.registry
            .write()
            .await
            .db_servers
            .insert(server.fields.server_id.clone(), server);
    }

    async fn server(&self, server_id: &str) -> Option<MCPServer> {
        self.registry
            .read()
            .await
            .config_servers
            .get(server_id)
            .cloned()
    }

    async fn servers(&self) -> Vec<MCPServer> {
        self.registry
            .read()
            .await
            .config_servers
            .values()
            .cloned()
            .collect()
    }

    async fn server_named(&self, server_name: &str) -> Option<MCPServer> {
        self.servers()
            .await
            .into_iter()
            .find(|server| server.fields.server_name.as_deref() == Some(server_name))
    }
}

fn settings_with_startup(on_startup: bool) -> DiscoverySettings {
    let defaults = DiscoverySettings::default();
    DiscoverySettings {
        on_startup,
        retry_delays: defaults.retry_delays,
        backoff_base: defaults.backoff_base,
        backoff_cap: defaults.backoff_cap,
        temporary_ttl: defaults.temporary_ttl,
        fetch: defaults.fetch,
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

fn config(source: &str) -> IndexMap<String, McpServerConfig> {
    serde_json::from_str(source).unwrap()
}

fn aliases(source: &str) -> IndexMap<String, String> {
    serde_json::from_str(source).unwrap()
}

fn json_server(source: &str) -> MCPServer {
    serde_json::from_str(source).unwrap()
}

fn corpus(name: &str) -> String {
    format!(
        "{}/fuzz/corpus/openapi_spec/{name}",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn secret_text(credential: &SecretCredential) -> String {
    credential.0.0.expose_secret().to_owned()
}

fn mcp_info(server: &MCPServer) -> Json {
    let encoded = serde_json::to_string(&server.fields.mcp_info).unwrap();
    serde_json::from_str(&encoded).unwrap()
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

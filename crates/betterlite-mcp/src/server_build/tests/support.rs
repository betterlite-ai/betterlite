#[derive(Default)]
struct FakeDecryptor {
    keys: Mutex<Vec<String>>,
}

impl FakeDecryptor {
    fn keys(&self) -> Vec<String> {
        self.keys.lock().unwrap().clone()
    }
}

impl Decryptor for FakeDecryptor {
    fn decrypt(&self, value: &str, key: &str) -> String {
        self.keys.lock().unwrap().push(key.to_owned());
        value.strip_prefix("enc:").unwrap_or(value).to_owned()
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
struct FakeSlots(Mutex<Vec<(String, bool)>>);

impl FakeSlots {
    fn entries(&self) -> Vec<(String, bool)> {
        self.0.lock().unwrap().clone()
    }
}

impl DeferredSlots for FakeSlots {
    fn record_deferred(&self, server_id: &str, discovery_deferred: bool) {
        self.0
            .lock()
            .unwrap()
            .push((server_id.to_owned(), discovery_deferred));
    }
}

#[derive(Default)]
struct FakeFetcher {
    body: String,
    calls: Mutex<Vec<String>>,
}

impl FakeFetcher {
    fn answering(body: &str) -> Self {
        Self {
            body: body.to_owned(),
            calls: Mutex::default(),
        }
    }

    fn urls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl MetadataFetcher for FakeFetcher {
    fn get_json(
        &self,
        url: &str,
        _headers: &[(String, String)],
        _timeout: Duration,
    ) -> impl Future<Output = Result<FetchedDocument, FetchError>> + Send {
        self.calls.lock().unwrap().push(url.to_owned());
        ready(Ok(FetchedDocument {
            status: StatusCode::OK,
            headers: HeaderMap::new(),
            body: self.body.clone().into_bytes(),
        }))
    }
}

struct Harness {
    decryptor: FakeDecryptor,
    cipher: DbCipher,
    fetcher: FakeFetcher,
    settings: DiscoverySettings,
    stdio: FakeStdioPolicy,
    slots: FakeSlots,
    prefixes: PrefixPolicy,
}

impl Harness {
    fn builder(&self) -> ServerBuilder<'_, FakeDecryptor, FakeFetcher, FakeStdioPolicy, FakeSlots> {
        ServerBuilder {
            decryptor: &self.decryptor,
            env_cipher: &self.cipher,
            fetcher: &self.fetcher,
            settings: &self.settings,
            stdio: &self.stdio,
            slots: &self.slots,
            prefixes: self.prefixes.clone(),
        }
    }

    async fn build(&self, table: &McpServerTable) -> Result<MCPServer, BuildError> {
        self.build_with(table, BuildOptions::default()).await
    }

    async fn build_with(
        &self,
        table: &McpServerTable,
        options: BuildOptions,
    ) -> Result<MCPServer, BuildError> {
        self.builder().build_from_table(table, options).await
    }
}

fn harness() -> Harness {
    Harness {
        decryptor: FakeDecryptor::default(),
        cipher: test_cipher(),
        fetcher: FakeFetcher::default(),
        settings: settings_with_startup(false),
        stdio: FakeStdioPolicy::default(),
        slots: FakeSlots::default(),
        prefixes: PrefixPolicy::default(),
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

fn encrypt(cipher: &DbCipher, plaintext: &str) -> String {
    let sealed = cipher
        .encrypt_value(StoredValue::<()>::Text(SecretString::from(plaintext)), None)
        .unwrap();
    match sealed {
        StoredValue::Text(text) => text.expose_secret().clone(),
        StoredValue::Other(()) => String::new(),
    }
}

fn env_value(entry: &BTreeMap<String, JsonValue>) -> Option<&JsonValue> {
    entry.get("value")
}

fn json(text: &str) -> JsonValue {
    serde_json::from_str(text).unwrap()
}

fn secret_text(credential: &Option<SecretCredential>) -> Option<String> {
    credential
        .as_ref()
        .map(|secret| secret.0.0.expose_secret().clone())
}

fn row(source: &str) -> McpServerTable {
    serde_json::from_str(source).unwrap()
}

fn server(source: &str) -> MCPServer {
    serde_json::from_str(source).unwrap()
}

fn oauth_request() -> TableOAuthRequest<'static> {
    TableOAuthRequest {
        server_id: "test-server",
        server_ref: "test-server",
        server_url: None,
        auth_type: None,
        oauth2_flow: None,
        dcr_bridge: None,
        manual_issuer: None,
        manual_authorization_url: None,
        manual_token_url: None,
        manual_registration_url: None,
        is_discovery_auth_type: false,
        use_issuer_anchor: false,
        scopes: None,
        token_exchange_endpoint: None,
    }
}

fn metadata_document(issuer: &str) -> String {
    format!(
        "{{\"issuer\":\"{issuer}\",\"authorization_endpoint\":\"{issuer}/authorize\",\
         \"token_endpoint\":\"{issuer}/token\",\"registration_endpoint\":\"{issuer}/register\",\
         \"scopes_supported\":[\"read\"],\"authorization_servers\":[\"{issuer}\"]}}"
    )
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
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn capture() -> (Captured, DefaultGuard) {
    capture_at(Level::WARN)
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

fn all_short_prefixes(server_id: &str) -> BTreeMap<String, String> {
    let mut taken = BTreeMap::new();
    for attempt in 0..1024_u32 {
        if let Ok(prefix) = compute_short_server_prefix(server_id, attempt) {
            taken.insert(prefix, String::from("other-server"));
        }
    }
    taken
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

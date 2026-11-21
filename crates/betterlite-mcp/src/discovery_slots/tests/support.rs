const SERVER_URL: &str = "https://mcp.test/mcp";
const MOVED_URL: &str = "https://moved.test/mcp";
const RESOURCE_URL: &str = "https://mcp.test/.well-known/oauth-protected-resource/mcp";
const AUTHORIZATION_SERVER_URL: &str = "https://idp.test/.well-known/oauth-authorization-server";
const RESOURCE_DOCUMENT: &str = r#"{"authorization_servers":["https://idp.test"]}"#;
const AUTHORIZATION_DOCUMENT: &str = r#"{"issuer":"https://idp.test","authorization_endpoint":"https://idp.test/authorize","token_endpoint":"https://idp.test/token","registration_endpoint":"https://idp.test/register","scopes_supported":["read"]}"#;

#[derive(Clone, Copy, Debug)]
enum Answer {
    Document(StatusCode, &'static str),
    Transport,
    Hang,
}

const EMPTY_DOCUMENT: Answer = Answer::Document(StatusCode::OK, "{}");

#[derive(Default)]
struct ScriptedFetcher {
    scripts: Mutex<BTreeMap<String, VecDeque<Answer>>>,
    calls: Mutex<Vec<(String, Instant)>>,
    gate: Option<Arc<Semaphore>>,
}

impl ScriptedFetcher {
    fn new(scripts: Vec<(&str, Vec<Answer>)>) -> Self {
        let fetcher = Self::default();
        for (url, answers) in scripts {
            fetcher
                .scripts
                .lock()
                .unwrap()
                .insert(url.to_owned(), VecDeque::from(answers));
        }
        fetcher
    }

    fn discoverable(server_answers: Vec<Answer>) -> Self {
        Self::new(vec![
            (SERVER_URL, server_answers),
            (
                RESOURCE_URL,
                vec![Answer::Document(StatusCode::OK, RESOURCE_DOCUMENT)],
            ),
            (
                AUTHORIZATION_SERVER_URL,
                vec![Answer::Document(StatusCode::OK, AUTHORIZATION_DOCUMENT)],
            ),
        ])
    }

    fn with_gate(mut self, gate: &Arc<Semaphore>) -> Self {
        self.gate = Some(Arc::clone(gate));
        self
    }

    fn call_times(&self, url: &str) -> Vec<Instant> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(called, _)| called == url)
            .map(|(_, at)| *at)
            .collect()
    }

    fn calls_to(&self, url: &str) -> usize {
        self.call_times(url).len()
    }

    fn called_urls(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .map(|(called, _)| called.clone())
            .collect()
    }

    fn next_answer(&self, url: &str) -> Answer {
        let mut scripts = self.scripts.lock().unwrap();
        let Some(answers) = scripts.get_mut(url) else {
            return Answer::Document(StatusCode::NOT_FOUND, "");
        };
        if answers.len() > 1 {
            answers.pop_front().unwrap()
        } else {
            *answers.front().unwrap()
        }
    }
}

impl MetadataFetcher for Arc<ScriptedFetcher> {
    fn get_json(
        &self,
        url: &str,
        _headers: &[(String, String)],
        _timeout: Duration,
    ) -> impl Future<Output = Result<FetchedDocument, FetchError>> + Send {
        self.calls
            .lock()
            .unwrap()
            .push((url.to_owned(), Instant::now()));
        let answer = self.next_answer(url);
        let gate = self.gate.clone();
        async move {
            if let Some(held) = gate {
                drop(held.acquire().await.unwrap());
            }
            match answer {
                Answer::Document(status, body) => Ok(FetchedDocument {
                    status,
                    headers: HeaderMap::new(),
                    body: body.as_bytes().to_vec(),
                }),
                Answer::Transport => Err(FetchError {
                    message: String::from("connection refused"),
                }),
                Answer::Hang => pending().await,
            }
        }
    }
}

#[derive(Clone)]
struct FakeClock(Arc<Mutex<Instant>>);

impl FakeClock {
    fn advance(&self, by: Duration) {
        let mut now = self.0.lock().unwrap();
        *now = now.checked_add(by).unwrap();
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Instant {
        *self.0.lock().unwrap()
    }
}

struct Harness {
    slots: Arc<DiscoverySlots<Arc<ScriptedFetcher>, FakeClock>>,
    fetcher: Arc<ScriptedFetcher>,
    clock: FakeClock,
    registry: SharedRegistry,
}

fn build_harness(
    fetcher: ScriptedFetcher,
    registry: SharedRegistry,
    settings: DiscoverySettings,
) -> Harness {
    let shared = Arc::new(fetcher);
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let slots = DiscoverySlots::new(
        Arc::clone(&registry),
        Arc::clone(&shared),
        clock.clone(),
        settings,
        TaskTracker::new(),
    );
    Harness {
        slots: Arc::new(slots),
        fetcher: shared,
        clock,
        registry,
    }
}

fn litellm_delays() -> Vec<Duration> {
    vec![Duration::from_millis(50), Duration::from_millis(150)]
}

const fn settings_with(on_startup: bool, retry_delays: Vec<Duration>) -> DiscoverySettings {
    DiscoverySettings {
        on_startup,
        retry_delays,
        backoff_base: Duration::from_secs(30),
        backoff_cap: Duration::from_secs(900),
        temporary_ttl: Duration::from_secs(300),
        fetch: FetchSettings {
            metadata_timeout: Duration::from_secs(10),
        },
    }
}

fn registry_of(config_servers: Vec<MCPServer>, db_servers: Vec<MCPServer>) -> SharedRegistry {
    let settings = RegistrySettings {
        public_mcp_servers: Vec::new(),
        public_hub_strict: false,
        internal_ip_ranges: None,
    };
    Arc::new(RwLock::new(ServerRegistry::new(
        settings,
        config_servers,
        db_servers,
    )))
}

fn server(server_id: &str, auth_type: &str) -> MCPServer {
    serde_json::from_str(&format!(
        r#"{{"server_id":"{server_id}","name":"{server_id}","url":"{SERVER_URL}","transport":"http","auth_type":"{auth_type}"}}"#
    ))
    .unwrap()
}

fn with_endpoints(mut server: MCPServer) -> MCPServer {
    server.fields.authorization_url = Some(String::from("https://idp.test/authorize"));
    server.fields.token_url = Some(String::from("https://idp.test/token"));
    server
}

fn discovered(server: MCPServer) -> MCPServer {
    let mut resolved = with_endpoints(server);
    resolved.fields.registration_url = Some(String::from("https://idp.test/register"));
    resolved.fields.scopes = Some(vec![String::from("read")]);
    resolved.fields.issuer = Some(String::from("https://idp.test"));
    resolved
}

fn incomplete(server_ref: &str) -> Result<MCPServer, DiscoveryUnavailable> {
    Err(DiscoveryUnavailable::Failed {
        reason: FailureReason::Incomplete,
        server_ref: server_ref.to_owned(),
    })
}

async fn registered_db(harness: &Harness, server_id: &str) -> MCPServer {
    let registry = harness.registry.read().await;
    registry.db_servers.get(server_id).cloned().unwrap()
}

async fn wait_for_calls(fetcher: &ScriptedFetcher, url: &str, count: usize) {
    while fetcher.calls_to(url) < count {
        yield_now().await;
    }
}

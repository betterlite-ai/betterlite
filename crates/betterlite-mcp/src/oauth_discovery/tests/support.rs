#[derive(Clone, Debug)]
enum FakeAnswer {
    Document {
        status: StatusCode,
        headers: Vec<(String, String)>,
        body: String,
    },
    Transport(String),
}

fn document(body: &str) -> FakeAnswer {
    FakeAnswer::Document {
        status: StatusCode::OK,
        headers: Vec::new(),
        body: body.to_owned(),
    }
}

fn status_document(status: StatusCode) -> FakeAnswer {
    FakeAnswer::Document {
        status,
        headers: Vec::new(),
        body: String::new(),
    }
}

fn challenge_document(value: &str) -> FakeAnswer {
    FakeAnswer::Document {
        status: StatusCode::UNAUTHORIZED,
        headers: vec![(String::from("www-authenticate"), value.to_owned())],
        body: String::new(),
    }
}

fn challenge_fields_document(headers: Vec<(&'static str, &'static str)>) -> FakeAnswer {
    FakeAnswer::Document {
        status: StatusCode::UNAUTHORIZED,
        headers: headers
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect(),
        body: String::new(),
    }
}

fn transport_error(message: &str) -> FakeAnswer {
    FakeAnswer::Transport(message.to_owned())
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FetchCall {
    url: String,
    headers: Vec<(String, String)>,
    timeout: Duration,
}

#[derive(Default)]
struct FakeFetcher {
    answers: BTreeMap<String, FakeAnswer>,
    calls: Mutex<Vec<FetchCall>>,
}

impl FakeFetcher {
    fn new(answers: Vec<(&'static str, FakeAnswer)>) -> Self {
        let mut fetcher = Self::default();
        for (url, answer) in answers {
            fetcher.answers.insert(url.to_owned(), answer);
        }
        fetcher
    }

    fn urls(&self) -> Vec<String> {
        self.calls().into_iter().map(|call| call.url).collect()
    }

    fn calls(&self) -> Vec<FetchCall> {
        self.calls.lock().unwrap().clone()
    }

    fn record(&self, url: &str, headers: &[(String, String)], timeout: Duration) {
        self.calls.lock().unwrap().push(FetchCall {
            url: url.to_owned(),
            headers: headers.to_vec(),
            timeout,
        });
    }

    fn respond(&self, url: &str) -> Result<FetchedDocument, FetchError> {
        match self.answers.get(url) {
            Some(FakeAnswer::Document {
                status,
                headers,
                body,
            }) => Ok(FetchedDocument {
                status: *status,
                headers: header_map(headers),
                body: body.as_bytes().to_vec(),
            }),
            Some(FakeAnswer::Transport(message)) => Err(FetchError {
                message: message.clone(),
            }),
            None => Err(FetchError {
                message: format!("no canned answer for {url}"),
            }),
        }
    }
}

fn header_map(headers: &[(String, String)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (header_name, header_value) in headers {
        let name = HeaderName::from_bytes(header_name.as_bytes()).unwrap();
        let value = HeaderValue::from_bytes(header_value.as_bytes()).unwrap();
        map.append(name, value);
    }
    map
}

impl MetadataFetcher for FakeFetcher {
    fn get_json(
        &self,
        url: &str,
        headers: &[(String, String)],
        timeout: Duration,
    ) -> impl Future<Output = Result<FetchedDocument, FetchError>> + Send {
        self.record(url, headers, timeout);
        ready(self.respond(url))
    }
}

fn settings() -> DiscoverySettings {
    DiscoverySettings::default()
}

fn settings_with_timeout(timeout: Duration) -> DiscoverySettings {
    DiscoverySettings {
        metadata_timeout: timeout,
    }
}

fn request(server_url: &str) -> DiscoveryRequest<'_> {
    DiscoveryRequest {
        server_url,
        allow_origin_fallback: true,
        warn_when_no_metadata: false,
    }
}

fn quiet_request(server_url: &str) -> DiscoveryRequest<'_> {
    DiscoveryRequest {
        server_url,
        allow_origin_fallback: false,
        warn_when_no_metadata: false,
    }
}

fn warning_request(server_url: &str) -> DiscoveryRequest<'_> {
    DiscoveryRequest {
        server_url,
        allow_origin_fallback: true,
        warn_when_no_metadata: true,
    }
}

fn authorization_server_document(issuer: &str) -> String {
    format!(
        "{{\"issuer\":\"{issuer}\",\"authorization_endpoint\":\"{issuer}/authorize\",\
         \"token_endpoint\":\"{issuer}/token\",\"registration_endpoint\":\"{issuer}/register\",\
         \"scopes_supported\":[\"read\"]}}"
    )
}

fn resource_document(servers: &[&str], scopes: Option<&[&str]>) -> String {
    let server_list = quoted_list(servers);
    scopes.map(quoted_list).map_or_else(
        || format!("{{\"authorization_servers\":[{server_list}]}}"),
        |scope_list| {
            format!(
                "{{\"authorization_servers\":[{server_list}],\"scopes_supported\":[{scope_list}]}}"
            )
        },
    )
}

fn quoted_list(values: &[&str]) -> String {
    values
        .iter()
        .map(|value| format!("\"{value}\""))
        .collect::<Vec<String>>()
        .join(",")
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
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
        self.0
            .lock()
            .map_err(|_error| io::Error::other("the captured log lock is poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn capture() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_max_level(tracing::Level::WARN)
        .with_ansi(false)
        .finish();
    (captured, set_default(subscriber))
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct SeenHttp {
    method: String,
    uri: String,
    headers: Vec<(String, String)>,
    body: String,
}

#[derive(Clone)]
struct HttpState {
    seen: Arc<Mutex<Vec<SeenHttp>>>,
    status: u16,
    body: &'static str,
    content_type: Option<&'static str>,
}

struct HttpServer {
    base: String,
    seen: Arc<Mutex<Vec<SeenHttp>>>,
    _tasks: JoinSet<()>,
}

async fn record_http(
    State(state): State<HttpState>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: String,
) -> Response {
    let mut recorded = Vec::with_capacity(headers.len());
    for (name, value) in &headers {
        recorded.push((name.as_str().to_owned(), value.to_str().unwrap().to_owned()));
    }
    state.seen.lock().unwrap().push(SeenHttp {
        method: method.as_str().to_owned(),
        uri: uri.to_string(),
        headers: recorded,
        body,
    });
    let status = StatusCode::from_u16(state.status).unwrap();
    let mut response = (status, state.body).into_response();
    if let Some(content_type) = state.content_type {
        response.headers_mut().insert(
            HeaderName::from_static("content-type"),
            HeaderValue::from_static(content_type),
        );
    }
    response.headers_mut().insert(
        HeaderName::from_static("x-ms-error-code"),
        HeaderValue::from_static("denied"),
    );
    response
}

async fn http_server(
    status: u16,
    body: &'static str,
    content_type: Option<&'static str>,
) -> HttpServer {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let state = HttpState {
        seen: Arc::clone(&seen),
        status,
        body,
        content_type,
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let application = Router::new()
        .route("/{*path}", any(record_http))
        .with_state(state);
    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        axum::serve(listener, application).await.unwrap();
    });
    HttpServer {
        base,
        seen,
        _tasks: tasks,
    }
}

fn azure_http() -> AzureHttp {
    AzureHttp {
        runtime: Handle::current(),
        client: proxied_client(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn azure_http_the_client_secret_transport_posts_the_form() {
    let server = http_server(200, r#"{"access_token":"wire-token"}"#, None).await;
    let transport = azure_http();
    let url = format!("{}/tenant/oauth2/v2.0/token", server.base);
    let reply = run_blocking(move || {
        ClientSecretTransport::send(
            &transport,
            &HttpRequest {
                method: HttpMethod::Post,
                url,
                query: vec![("client_info".to_owned(), "1".to_owned())],
                headers: vec![("x-client-request-id".to_owned(), "request-1".to_owned())],
                form: vec![("grant_type".to_owned(), "client_credentials".to_owned())],
            },
        )
    })
    .await
    .unwrap();
    assert_eq!(reply.status, 200);
    assert_eq!(
        reply.body.expose_secret(),
        r#"{"access_token":"wire-token"}"#
    );
    let seen = server.seen.lock().unwrap().clone();
    let request = &seen[0];
    assert_eq!(request.method, "POST");
    assert_eq!(request.uri, "/tenant/oauth2/v2.0/token?client_info=1");
    assert!(request.body.contains("grant_type=client_credentials"));
    assert!(
        request
            .headers
            .contains(&("x-client-request-id".to_owned(), "request-1".to_owned()))
    );
    assert!(request.headers.contains(&(
        "content-type".to_owned(),
        "application/x-www-form-urlencoded".to_owned()
    )));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn azure_http_authority_discovery_sends_no_body() {
    let server = http_server(200, "{}", None).await;
    let transport = azure_http();
    let url = format!(
        "{}/tenant/v2.0/.well-known/openid-configuration",
        server.base
    );
    let reply = run_blocking(move || {
        ClientSecretTransport::send(
            &transport,
            &HttpRequest {
                method: HttpMethod::Get,
                url,
                query: Vec::new(),
                headers: Vec::new(),
                form: Vec::new(),
            },
        )
    })
    .await
    .unwrap();
    assert_eq!(reply.status, 200);
    let seen = server.seen.lock().unwrap().clone();
    assert_eq!(seen[0].method, "GET");
    assert!(seen[0].body.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn azure_http_the_password_transport_posts_the_form() {
    let server = http_server(200, "{}", None).await;
    let transport = azure_http();
    let url = format!("{}/tenant/oauth2/v2.0/token", server.base);
    let reply = run_blocking(move || {
        PasswordTransport::send(
            &transport,
            &PasswordRequest {
                method: HttpMethod::Post,
                url,
                query: Vec::new(),
                headers: Vec::new(),
                form: vec![("username".to_owned(), "user@example.com".to_owned())],
            },
        )
    })
    .await
    .unwrap();
    assert_eq!(reply.status, 200);
    let seen = server.seen.lock().unwrap().clone();
    assert_eq!(seen[0].method, "POST");
    assert!(seen[0].body.contains("username=user%40example.com"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn azure_http_the_imds_transport_returns_the_status_and_headers() {
    let server = http_server(403, r#"{"error":"denied"}"#, None).await;
    let transport = azure_http();
    let url = format!("{}/metadata/identity/oauth2/token", server.base);
    let reply = run_blocking(move || {
        ImdsTransport::get(
            &transport,
            &MetadataRequest {
                url,
                query: vec![("resource".to_owned(), "https://redis.azure.com/".to_owned())],
                headers: vec![("Metadata".to_owned(), SecretString::from("true"))],
                timeout: Duration::from_secs(2),
            },
        )
    })
    .await
    .unwrap();
    assert_eq!(reply.status, 403);
    assert_eq!(reply.body.expose_secret(), r#"{"error":"denied"}"#);
    assert!(
        reply
            .headers
            .contains(&("x-ms-error-code".to_owned(), "denied".to_owned()))
    );
    let seen = server.seen.lock().unwrap().clone();
    assert!(
        seen[0]
            .headers
            .contains(&("metadata".to_owned(), "true".to_owned()))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn azure_http_the_arc_transport_carries_its_secret_header() {
    let server = http_server(200, r#"{"access_token":"arc-token"}"#, None).await;
    let transport = azure_http();
    let url = format!("{}/metadata/identity/oauth2/token", server.base);
    let reply = run_blocking(move || {
        ArcTransport::get(
            &transport,
            &MetadataRequest {
                url,
                query: vec![
                    ("api-version".to_owned(), "2020-06-01".to_owned()),
                    ("resource".to_owned(), "r".to_owned()),
                ],
                headers: vec![(
                    "Authorization".to_owned(),
                    SecretString::from("Basic secret".to_owned()),
                )],
                timeout: Duration::from_secs(5),
            },
        )
    })
    .await
    .unwrap();
    assert_eq!(reply.status, 200);
    assert_eq!(
        reply.body.expose_secret(),
        r#"{"access_token":"arc-token"}"#
    );
    let seen = server.seen.lock().unwrap().clone();
    assert_eq!(
        seen[0].uri,
        "/metadata/identity/oauth2/token?api-version=2020-06-01&resource=r"
    );
    assert!(
        seen[0]
            .headers
            .contains(&("authorization".to_owned(), "Basic secret".to_owned()))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn azure_http_the_hosted_transport_returns_the_content_type() {
    let server = http_server(
        200,
        r#"{"access_token":"hosted"}"#,
        Some("application/json"),
    )
    .await;
    let transport = azure_http();
    let url = format!("{}/msi/token", server.base);
    let reply = run_blocking(move || {
        HostedTransport::send(
            &transport,
            &HostedRequest {
                method: HttpMethod::Get,
                url,
                query: vec![("api-version".to_owned(), "2019-08-01".to_owned())],
                headers: vec![("secret", SecretString::from("value".to_owned()))],
                form: Vec::new(),
            },
        )
    })
    .await
    .unwrap();
    assert_eq!(reply.content_type.as_deref(), Some("application/json"));
    assert_eq!(reply.body.expose_secret(), r#"{"access_token":"hosted"}"#);
    let seen = server.seen.lock().unwrap().clone();
    assert!(
        seen[0]
            .headers
            .contains(&("secret".to_owned(), "value".to_owned()))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn azure_http_the_hosted_transport_rejects_a_non_success_status() {
    let server = http_server(500, "failure", None).await;
    let transport = azure_http();
    let url = format!("{}/msi/token", server.base);
    let error = run_blocking(move || {
        HostedTransport::send(
            &transport,
            &HostedRequest {
                method: HttpMethod::Get,
                url,
                query: Vec::new(),
                headers: Vec::new(),
                form: Vec::new(),
            },
        )
    })
    .await
    .unwrap_err();
    assert!(error.to_string().contains("500"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn azure_http_the_oidc_transport_posts_the_form() {
    let server = http_server(200, r#"{"access_token":"oidc"}"#, None).await;
    let transport = azure_http();
    let url = format!("{}/tenant/oauth2/v2.0/token", server.base);
    let reply = run_blocking(move || {
        OidcTransport::post_form(
            &transport,
            &url,
            &[(
                "client_assertion".to_owned(),
                SecretString::from("assertion".to_owned()),
            )],
        )
    })
    .await
    .unwrap();
    assert_eq!(reply.status, 200);
    assert_eq!(reply.body.expose_secret(), r#"{"access_token":"oidc"}"#);
    let seen = server.seen.lock().unwrap().clone();
    assert!(seen[0].body.contains("client_assertion=assertion"));
}

async fn latin_one_header(State(name): State<HeaderName>) -> Response {
    let mut response = StatusCode::OK.into_response();
    response
        .headers_mut()
        .insert(name, HeaderValue::from_bytes(b"\xe9").unwrap());
    response
}

async fn latin_one_server(header: &'static str) -> (String, JoinSet<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let application = Router::new()
        .route("/{*path}", any(latin_one_header))
        .with_state(HeaderName::from_static(header));
    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        axum::serve(listener, application).await.unwrap();
    });
    (base, tasks)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn azure_http_a_repeated_request_header_keeps_every_value() {
    let server = http_server(200, "{}", None).await;
    let transport = azure_http();
    let url = format!("{}/metadata/identity/oauth2/token", server.base);
    run_blocking(move || {
        ImdsTransport::get(
            &transport,
            &MetadataRequest {
                url,
                query: Vec::new(),
                headers: vec![
                    ("x-forwarded".to_owned(), SecretString::from("first")),
                    ("x-forwarded".to_owned(), SecretString::from("second")),
                ],
                timeout: Duration::from_secs(2),
            },
        )
    })
    .await
    .unwrap();
    let seen = server.seen.lock().unwrap().clone();
    assert!(
        seen[0]
            .headers
            .contains(&("x-forwarded".to_owned(), "first".to_owned()))
    );
    assert!(
        seen[0]
            .headers
            .contains(&("x-forwarded".to_owned(), "second".to_owned()))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn azure_http_a_response_header_outside_ascii_is_read_as_latin_1() {
    let (base, _tasks) = latin_one_server("x-note").await;
    let transport = azure_http();
    let url = format!("{base}/metadata/identity/oauth2/token");
    let reply = run_blocking(move || {
        ImdsTransport::get(
            &transport,
            &MetadataRequest {
                url,
                query: Vec::new(),
                headers: Vec::new(),
                timeout: Duration::from_secs(2),
            },
        )
    })
    .await
    .unwrap();
    assert!(
        reply
            .headers
            .contains(&("x-note".to_owned(), "\u{e9}".to_owned()))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn azure_http_a_content_type_outside_ascii_is_read_as_latin_1() {
    let (base, _tasks) = latin_one_server("content-type").await;
    let transport = azure_http();
    let url = format!("{base}/msi/token");
    let reply = run_blocking(move || {
        HostedTransport::send(
            &transport,
            &HostedRequest {
                method: HttpMethod::Get,
                url,
                query: Vec::new(),
                headers: Vec::new(),
                form: Vec::new(),
            },
        )
    })
    .await
    .unwrap();
    assert_eq!(reply.content_type.as_deref(), Some("\u{e9}"));
}

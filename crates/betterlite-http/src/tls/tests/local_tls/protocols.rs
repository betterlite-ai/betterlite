use std::collections::BTreeMap;
use std::fs;
use std::future::poll_fn;
use std::io::ErrorKind;
use std::net::TcpListener as StdListener;
use std::sync::Arc;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::TlsPolicy;
use betterlite_config::types::ProxyTrust;
use bytes::Bytes;
use h2::server::handshake;
use http::{Response, Version};
use rstest::rstest;
use rustls::ServerConfig;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Builder;
use tokio::time::timeout;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::server::TlsStream;

use super::{TEST_CA, TIMEOUT, server_config};
use crate::proxy::ProxiedClient;
use crate::proxy::limits::TransportLimits;
use crate::proxy::pool::ClientPool;

#[rstest]
#[case::http1_default("default", Version::HTTP_11)]
#[case::http1_disabled("disabled", Version::HTTP_11)]
#[case::http2_enabled("enabled", Version::HTTP_2)]
#[case::http2_with_curve("curve", Version::HTTP_2)]
#[case::http2_from_configuration("configured", Version::HTTP_2)]
#[case::http2_falls_back_to_http1("fallback", Version::HTTP_11)]
fn http_policy_tests_behavior(#[case] mode: &str, #[case] expected: Version) {
    let ca_file = tempfile::NamedTempFile::new().unwrap();
    fs::write(ca_file.path(), TEST_CA).unwrap();
    let mut environment = BTreeMap::from([(
        "SSL_CERT_FILE".to_owned(),
        ca_file.path().to_str().unwrap().to_owned(),
    )]);
    if mode != "default" {
        environment.insert(
            "LITELLM_HTTP2".to_owned(),
            (!matches!(mode, "disabled" | "configured")).to_string(),
        );
    }
    if mode == "curve" {
        environment.insert("SSL_ECDH_CURVE".to_owned(), "X25519".to_owned());
    }
    let yaml = if mode == "configured" {
        "litellm_settings:\n  http2: true\n"
    } else {
        ""
    };
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(yaml)
    .unwrap();
    let policy = TlsPolicy::resolve(&config, &environment, None).unwrap();
    let routing = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &environment).unwrap());
    let client = ClientPool::new(1, TransportLimits::DEFAULT)
        .client(&policy, &routing)
        .unwrap();
    let listener = StdListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!(
        "https://localhost:{}/policy",
        listener.local_addr().unwrap().port()
    );
    let mut origin = server_config("").as_ref().clone();
    origin.alpn_protocols = if mode == "fallback" {
        vec![b"http/1.1".to_vec()]
    } else {
        vec![b"h2".to_vec(), b"http/1.1".to_vec()]
    };
    let shared_origin = Arc::new(origin);
    let runtime = Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(timeout_exchange(
        listener,
        shared_origin,
        client,
        url,
        expected,
    ));
}

async fn timeout_exchange(
    listener: StdListener,
    origin: Arc<ServerConfig>,
    client: ProxiedClient,
    url: String,
    expected: Version,
) {
    timeout(TIMEOUT, async move {
        let async_listener = TcpListener::from_std(listener).unwrap();
        let request = async {
            let response = client
                .client
                .get(&url)
                .timeout(TIMEOUT)
                .send()
                .await
                .unwrap();
            assert_eq!(response.version(), expected);
            assert_eq!(response.bytes().await.unwrap(), Bytes::from_static(b"ok"));
        };
        tokio::join!(serve(&async_listener, origin), request);
    })
    .await
    .unwrap();
}

async fn serve(listener: &TcpListener, origin: Arc<ServerConfig>) {
    let (socket, _) = listener.accept().await.unwrap();
    let stream = TlsAcceptor::from(origin).accept(socket).await.unwrap();
    match stream.get_ref().1.alpn_protocol() {
        Some(b"h2") => http2_response(stream).await,
        Some(b"http/1.1") => http1_response(stream).await,
        protocol => panic!("unexpected negotiated protocol: {protocol:?}"),
    }
}

async fn http2_response(stream: TlsStream<TcpStream>) {
    let mut connection = handshake(stream).await.unwrap();
    let (request, mut response) = connection.accept().await.unwrap().unwrap();
    assert_eq!(request.version(), Version::HTTP_2);
    assert_eq!(request.uri().path(), "/policy");
    response
        .send_response(Response::new(()), false)
        .unwrap()
        .send_data(Bytes::from_static(b"ok"), true)
        .unwrap();
    connection.graceful_shutdown();
    if let Err(error) = poll_fn(|context| connection.poll_closed(context)).await {
        // reqwest can close the socket before the final GOAWAY is written.
        assert_eq!(error.get_io().unwrap().kind(), ErrorKind::BrokenPipe);
    }
}

async fn http1_response(mut stream: TlsStream<TcpStream>) {
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        assert!(head.len() < 8192);
        head.push(stream.read_u8().await.unwrap());
    }
    assert!(head.starts_with(b"GET /policy HTTP/1.1\r\n"));
    stream
        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
        .await
        .unwrap();
    stream.shutdown().await.unwrap();
}

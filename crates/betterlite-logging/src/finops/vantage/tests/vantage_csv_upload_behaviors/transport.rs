use std::collections::BTreeMap;
use std::future::pending;
use std::str::from_utf8;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::{TlsPolicy, Verification};
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::pool::ClientPool;
use betterlite_http::proxy::transport_error::TransportError;
use secrecy::SecretString;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

use crate::finops::vantage::destination::VantageDestination;
use crate::finops::vantage::error::DeliveryError;
use crate::finops::vantage::sender::{DEFAULT_UPLOAD_TIMEOUT, HttpCsvSender, UploadError};
use crate::finops::vantage::settings::VantageSettings;
use crate::finops::vantage::tests::support::{FILENAME, window};
use crate::focus_records::destinations::FocusDestination;

const CONTENT: &[u8] = b"ServiceName\r\nCompute\r\n";

fn destination(deadline: Duration, base_url: String) -> VantageDestination<HttpCsvSender> {
    let routing = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    let tls = TlsPolicy {
        verification: Verification::Enabled,
        ca_file: None,
        certificate: None,
        http2: false,
        ecdh_curve: None,
        user_agent: "betterlite".to_owned(),
    };
    let pool = ClientPool::new(1, TransportLimits::DEFAULT);
    let sender = HttpCsvSender {
        client: pool.client(&tls, &routing).unwrap(),
        deadline,
    };
    let settings = VantageSettings {
        api_key: Some(SecretString::from("key-1")),
        integration_token: Some(SecretString::from("token-1")),
        base_url: Some(base_url),
    };
    VantageDestination::new(sender, settings).unwrap()
}

async fn read_request(stream: &mut TcpStream) -> Vec<u8> {
    let mut received = Vec::new();
    let mut byte = [0_u8; 1];
    while !received.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte).await.unwrap();
        received.push(byte[0]);
    }
    let head = from_utf8(&received).unwrap().to_ascii_lowercase();
    let length = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length: "))
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = vec![0_u8; length];
    stream.read_exact(&mut body).await.unwrap();
    received.extend_from_slice(&body);
    received
}

async fn answer(listener: TcpListener) -> Vec<u8> {
    let (mut stream, _) = listener.accept().await.unwrap();
    let request = read_request(&mut stream).await;
    stream
        .write_all(b"HTTP/1.1 201 Created\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
        .await
        .unwrap();
    request
}

async fn stall(listener: TcpListener) {
    let (mut stream, _) = listener.accept().await.unwrap();
    let _request = read_request(&mut stream).await;
    pending::<()>().await;
}

#[tokio::test]
async fn the_destination_posts_the_multipart_csv_to_the_costs_url() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = answer(listener);
    let destination = destination(Duration::from_secs(5), format!("http://{address}"));
    let time_window = window();
    let upload = FocusDestination::deliver(&destination, CONTENT, &time_window, FILENAME);
    let (result, request) = tokio::join!(upload, server);
    result.unwrap();

    let text = String::from_utf8(request).unwrap();
    let (head, body) = text.split_once("\r\n\r\n").unwrap();
    assert!(
        head.starts_with("POST /v2/integrations/token-1/costs.csv HTTP/1.1\r\n"),
        "{head}"
    );
    assert!(
        head.to_ascii_lowercase()
            .contains("\r\nauthorization: bearer key-1"),
        "{head}"
    );
    let media_type = head
        .lines()
        .find_map(|line| line.strip_prefix("content-type: "))
        .unwrap();
    let boundary = media_type
        .strip_prefix("multipart/form-data; boundary=")
        .unwrap();
    assert_eq!(
        body,
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"csv\"; filename=\"usage.csv\"\r\nContent-Type: text/csv\r\n\r\nServiceName\r\nCompute\r\n\r\n--{boundary}--\r\n"
        )
    );
}

#[tokio::test]
async fn a_stalled_endpoint_ends_the_delivery_at_its_deadline() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut server = JoinSet::new();
    server.spawn(stall(listener));
    let destination = destination(Duration::from_millis(500), format!("http://{address}"));
    let result = FocusDestination::deliver(&destination, CONTENT, &window(), FILENAME).await;
    match &result {
        Err(DeliveryError::Send(UploadError::Transport(TransportError::Deadline(_)))) => {}
        other => panic!("the stalled upload was not ended by its deadline: {other:?}"),
    }
    server.shutdown().await;
}

#[test]
fn the_default_upload_deadline_is_litellms_request_timeout() {
    assert_eq!(DEFAULT_UPLOAD_TIMEOUT, Duration::from_secs(600));
}

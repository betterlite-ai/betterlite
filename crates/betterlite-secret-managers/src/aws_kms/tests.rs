#![cfg(test)]

use std::collections::BTreeMap;

use aws_sdk_kms::config::{Credentials, SharedCredentialsProvider, SharedHttpClient};
use aws_smithy_http_client::tls::{Provider, rustls_provider::CryptoMode};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;

pub(super) fn transport() -> SharedHttpClient {
    SharedHttpClient::new(
        aws_smithy_http_client::Builder::new()
            .tls_provider(Provider::Rustls(CryptoMode::AwsLc))
            .build_https(),
    )
}

pub(super) fn credentials() -> Option<SharedCredentialsProvider> {
    Some(SharedCredentialsProvider::new(Credentials::new(
        "test", "test", None, None, "test",
    )))
}

pub(super) fn kms_environment(listener: &TcpListener, region: &str) -> BTreeMap<String, String> {
    let address = listener.local_addr().unwrap();
    BTreeMap::from([
        (
            "AWS_ENDPOINT_URL_KMS".to_owned(),
            format!("http://{address}"),
        ),
        ("AWS_REGION_NAME".to_owned(), region.to_owned()),
    ])
}

pub(super) async fn serve(listener: &TcpListener, status: u16, body: &str) -> String {
    let (mut stream, _) = listener.accept().await.unwrap();
    let mut bytes = Vec::new();
    let header_end = loop {
        let mut chunk = [0; 4096];
        let count = stream.read(&mut chunk).await.unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&chunk[..count]);
        assert!(bytes.len() < 0x0001_0000);
        if let Some(position) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
    };
    let head = String::from_utf8(bytes[..header_end].to_vec())
        .unwrap()
        .to_lowercase();
    let length: usize = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length: "))
        .unwrap()
        .parse()
        .unwrap();
    while bytes.len() < header_end + length {
        let mut chunk = [0; 4096];
        let count = stream.read(&mut chunk).await.unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&chunk[..count]);
    }
    let response = format!(
        "HTTP/1.1 {status} Status\r\nContent-Type: application/x-amz-json-1.1\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await.unwrap();
    String::from_utf8(bytes).unwrap()
}

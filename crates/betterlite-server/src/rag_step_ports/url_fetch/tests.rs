#![cfg(test)]

use std::collections::BTreeMap;
use std::net::IpAddr;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::credentials::CredentialValues;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::user_url_policy::Settings;
use betterlite_config::types::ProxyTrust;
use betterlite_core::rag::ingestion::config::IngestionOptions;
use betterlite_core::rag::ingestion::credentials::{
    CredentialLookup as CredentialLookupPort, CredentialLookupError,
};
use betterlite_core::rag::ingestion::ocr::{OcrError, OcrOperation, OcrRequest, OcrResponse};
use betterlite_core::rag::ingestion::upload::UrlFetch as UrlFetchPort;
use betterlite_core::rag::ingestion::upload::UrlFetchError;
use betterlite_core::rag::ingestion::{IngestionPorts, IngestionState};
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::user_url::error::{SsrfError, UserUrlError};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;

use super::{UrlFetch, ssrf_checked};

struct NoOcr;

#[async_trait]
impl OcrOperation for NoOcr {
    async fn aocr(&self, _request: OcrRequest) -> Result<OcrResponse, OcrError> {
        Err(OcrError::Failed {
            message: "the upload test never runs OCR".to_owned(),
        })
    }
}

struct NoCredentials;

impl CredentialLookupPort for NoCredentials {
    fn credential_values(
        &self,
        credential_name: &str,
    ) -> Result<CredentialValues, CredentialLookupError> {
        Err(CredentialLookupError::NotFound {
            credential_name: credential_name.to_owned(),
        })
    }
}

fn client() -> ProxiedClient {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .expect("the policy resolves"),
    );
    ProxiedClient::outbound(&policy).expect("the client builds")
}

fn settings(validation: bool, allowed_hosts: &[&str]) -> Settings {
    Settings {
        user_url_validation: validation,
        user_url_allowed_hosts: allowed_hosts
            .iter()
            .map(|host| (*host).to_owned())
            .collect(),
    }
}

fn ingestion_state(fetch: UrlFetch) -> IngestionState {
    IngestionState::new(
        IngestionOptions::default(),
        None,
        IngestionPorts {
            ocr: Arc::new(NoOcr),
            uploader: Arc::new(fetch),
            credentials: Arc::new(NoCredentials),
        },
    )
    .expect("the ingestion state builds")
}

async fn serve_once(listener: TcpListener, reply: &[u8]) {
    let (mut socket, _) = listener.accept().await.expect("the test request arrives");
    let mut buffer = [0_u8; 2048];
    let _read = socket
        .read(&mut buffer)
        .await
        .expect("the request head is read");
    socket.write_all(reply).await.expect("the reply is written");
    socket.shutdown().await.expect("the reply is closed");
}

#[tokio::test]
async fn url_fetch_returns_the_bytes_filename_and_content_type() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("the listener binds");
    let address = listener.local_addr().expect("the listener address");
    let url = format!("http://{address}/files/report.pdf");
    let state = ingestion_state(ssrf_checked(client(), &settings(true, &["127.0.0.1"])));
    let reply = b"HTTP/1.1 200 OK\r\nContent-Type: application/pdf; charset=binary\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello";
    let (uploaded, ()) = tokio::join!(
        state.upload(None, Some(&url), None),
        serve_once(listener, reply)
    );
    let uploaded = uploaded.expect("the upload succeeds");
    assert_eq!(uploaded.filename.as_deref(), Some("report.pdf"));
    assert_eq!(uploaded.file_content.as_deref(), Some(b"hello".as_slice()));
    assert_eq!(
        uploaded.content_type.as_deref(),
        Some("application/pdf; charset=binary")
    );
    assert!(uploaded.existing_file_id.is_none());
}

#[tokio::test]
async fn url_fetch_refuses_a_blocked_address_with_the_ssrf_error() {
    let port = ssrf_checked(client(), &Settings::default());
    let error = port
        .fetch("http://127.0.0.1:9/files/report.pdf")
        .await
        .expect_err("the address is blocked");
    let UrlFetchError::Request(source) = error else {
        panic!("the block is a request error");
    };
    assert!(matches!(
        *source,
        UserUrlError::Ssrf(SsrfError::BlockedAddress { address })
            if address == IpAddr::from([127, 0, 0, 1])
    ));
}

#[tokio::test]
async fn url_fetch_skips_the_ssrf_check_when_the_policy_turns_validation_off() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("the listener binds");
    let address = listener.local_addr().expect("the listener address");
    let url = format!("http://{address}/plain.txt");
    let port = ssrf_checked(client(), &settings(false, &[]));
    let reply = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nlocal";
    let (fetched, ()) = tokio::join!(port.fetch(&url), serve_once(listener, reply));
    let fetched = fetched.expect("the unvalidated fetch reaches the local listener");
    assert_eq!(fetched.status, 200);
    assert_eq!(fetched.body, b"local");
}

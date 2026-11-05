#![cfg(test)]

use std::{collections::BTreeMap, future::IntoFuture as _, pin::Pin};

use aws_sdk_kms::config::{Credentials, SharedCredentialsProvider, SharedHttpClient};
use aws_smithy_http_client::tls::{Provider, rustls_provider::CryptoMode};
use axum::{
    Router,
    body::to_bytes,
    extract::{Request, State},
    http::{StatusCode, header::CONTENT_TYPE, request::Parts},
    response::{IntoResponse as _, Response},
};
use tokio::{
    net::TcpListener,
    sync::{mpsc, oneshot},
};

pub(super) struct KmsRequest {
    pub(super) head: Parts,
    pub(super) body: String,
}

#[derive(Clone)]
struct KmsAnswer {
    status: StatusCode,
    body: String,
    requests: mpsc::Sender<KmsRequest>,
}

pub(super) fn transport() -> SharedHttpClient {
    SharedHttpClient::new(
        aws_smithy_http_client::Builder::new()
            .tls_provider(Provider::Rustls(CryptoMode::AwsLc))
            .build_https(),
    )
}

pub(super) fn credentials() -> SharedCredentialsProvider {
    SharedCredentialsProvider::new(Credentials::new("test", "test", None, None, "test"))
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

async fn answer(State(kms): State<KmsAnswer>, request: Request) -> Response {
    let (head, request_body) = request.into_parts();
    let bytes = to_bytes(request_body, 0x0001_0000).await.unwrap();
    let body = String::from_utf8(bytes.to_vec()).unwrap();
    kms.requests.send(KmsRequest { head, body }).await.unwrap();
    (
        kms.status,
        [(CONTENT_TYPE, "application/x-amz-json-1.1")],
        kms.body,
    )
        .into_response()
}

pub(super) async fn with_kms<Call>(
    listener: TcpListener,
    status: u16,
    body: &str,
    call: Pin<Box<Call>>,
) -> (Call::Output, Vec<KmsRequest>)
where
    Call: Future,
{
    let (sender, mut receiver) = mpsc::channel(16);
    let (stop, stopped) = oneshot::channel();
    let router = Router::new().fallback(answer).with_state(KmsAnswer {
        status: StatusCode::from_u16(status).unwrap(),
        body: body.to_owned(),
        requests: sender,
    });
    let serving = axum::serve(listener, router)
        .with_graceful_shutdown(async { stopped.await.unwrap() })
        .into_future();
    let caller = async {
        let output = call.await;
        stop.send(()).unwrap();
        output
    };
    let (output, shutdown) = tokio::join!(caller, serving);
    shutdown.unwrap();
    let mut requests = Vec::new();
    while let Ok(request) = receiver.try_recv() {
        requests.push(request);
    }
    (output, requests)
}

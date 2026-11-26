use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::ProxiedClient;
use rstest::rstest;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;

use crate::jwt_keys::MAX_DOCUMENT_BYTES;
use crate::jwt_keys::http::{FetchError, JwksHttp as _, ProxiedJwksHttp};

fn client() -> ProxiedClient {
    let policy = Arc::new(ProxyPolicy {
        no_proxy: String::new(),
        primary: BTreeMap::new(),
        fallback: BTreeMap::new(),
        warnings: Vec::new(),
    });
    ProxiedClient::outbound(&policy).unwrap()
}

#[rstest]
#[case::default(MAX_DOCUMENT_BYTES, MAX_DOCUMENT_BYTES)]
#[case::smaller(4_usize, 4_usize)]
#[case::larger(MAX_DOCUMENT_BYTES + 1, MAX_DOCUMENT_BYTES)]
fn inherited_body_limit(#[case] inherited: usize, #[case] expected: usize) {
    let mut transport = client();
    transport.limits.body_bytes = inherited;
    assert_eq!(ProxiedJwksHttp::new(transport).body_limit, expected);
}

#[rstest]
#[case::within_limit(5_usize)]
#[case::over_limit(4_usize)]
#[tokio::test]
async fn proxied_response_and_limit(#[case] limit: usize) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/jwks", listener.local_addr().unwrap());
    let mut transport = client();
    transport.limits.body_bytes = limit;
    let http = ProxiedJwksHttp::new(transport);
    let answer = async {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = [0_u8; 1024];
        stream.read(&mut request).await.unwrap();
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello")
            .await
            .unwrap();
    };
    let (result, ()) = tokio::join!(http.get(&url), answer);
    if limit == 5 {
        let reply = result.unwrap();
        assert_eq!(reply.status, 200);
        assert_eq!(reply.body, b"hello");
    } else {
        assert!(matches!(
            result.unwrap_err(),
            FetchError::Failed(HttpError::TooLarge { limit: 4 })
        ));
    }
}

#[tokio::test]
async fn invalid_configured_url_is_transport() {
    let http = ProxiedJwksHttp::new(client());
    let error = http.get("not a URL").await.unwrap_err();
    assert!(matches!(
        error,
        FetchError::Transport(HttpError::Request(_))
    ));
}

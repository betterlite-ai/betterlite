#![cfg(test)]

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use crate::presidio_analysis::body::read_text_prefix;
use crate::presidio_analysis::error::PresidioAnalysisError;

async fn serve_once(response: &'static [u8]) -> (String, JoinSet<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut server = JoinSet::new();
    server.spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = [0_u8; 1024];
        let _read = stream.read(&mut request).await.unwrap();
        stream.write_all(response).await.unwrap();
    });
    (format!("http://{address}/analyze"), server)
}

#[tokio::test]
async fn response_text_refuses_a_body_that_is_not_utf8() {
    let (url, _server) = serve_once(
        b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 4\r\nConnection: close\r\n\r\nabc\xff",
    )
    .await;
    let response = reqwest::Client::default().get(url).send().await.unwrap();
    let error = read_text_prefix(response, 200).await.unwrap_err();
    assert!(matches!(
        error,
        PresidioAnalysisError::ResponseEncoding { .. }
    ));
    assert_eq!(error.status(), 500);
}

#[tokio::test]
async fn response_text_drops_a_character_cut_by_the_prefix_read() {
    let (url, _server) = serve_once(
        b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 4\r\nConnection: close\r\n\r\nabc\xc3",
    )
    .await;
    let response = reqwest::Client::default().get(url).send().await.unwrap();
    assert_eq!(read_text_prefix(response, 200).await.unwrap(), "abc");
}

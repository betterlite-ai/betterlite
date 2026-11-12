use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use indexmap::IndexMap;
use rstest::rstest;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};

use super::fixture::{Recorder, connection};
use crate::speech::call::{SpeechCall, audio_speech};
use crate::speech::request::{SpeechOptions, SpeechRequest};

async fn read_request_line(stream: &mut TcpStream) -> String {
    let mut received = Vec::new();
    while !received.ends_with(b"\r\n\r\n") {
        received.push(stream.read_u8().await.unwrap());
    }
    let head = String::from_utf8(received).unwrap();
    let length: usize = head
        .to_ascii_lowercase()
        .lines()
        .find_map(|line| line.strip_prefix("content-length: "))
        .map_or(0, |value| value.parse().unwrap());
    let mut body = vec![0_u8; length];
    stream.read_exact(&mut body).await.unwrap();
    head.lines().next().unwrap().to_owned()
}

async fn serve(listener: TcpListener, status: u16) -> Vec<String> {
    let (mut first, _first_peer) = listener.accept().await.unwrap();
    let mut lines = vec![read_request_line(&mut first).await];
    let moved = format!(
        "HTTP/1.1 {status} X\r\nlocation: /v1/relocated\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
    );
    first.write_all(moved.as_bytes()).await.unwrap();
    let (mut second, _second_peer) = listener.accept().await.unwrap();
    lines.push(read_request_line(&mut second).await);
    let audio = "HTTP/1.1 200 OK\r\ncontent-type: audio/mpeg\r\ncontent-length: 3\r\nconnection: close\r\n\r\n";
    second.write_all(audio.as_bytes()).await.unwrap();
    second.write_all(&[7, 8, 9]).await.unwrap();
    lines
}

#[rstest]
#[case::temporary_redirect_keeps_the_method(307, "POST /v1/relocated HTTP/1.1")]
#[case::found_becomes_a_get(302, "GET /v1/relocated HTTP/1.1")]
#[tokio::test]
async fn speech_follows_redirects(#[case] status: u16, #[case] second_line: &str) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut settings = connection();
    settings.api_base = Some(format!("http://{}/v1", listener.local_addr().unwrap()));
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    let transport = ProxiedClient::outbound(&policy).unwrap();
    let options = SpeechOptions {
        response_format: None,
        speed: None,
        instructions: None,
        extra_headers: IndexMap::new(),
    };
    let call = SpeechCall {
        request: SpeechRequest {
            model: "tts-1",
            input: "Hi",
            voice: "alloy",
            options: &options,
        },
        connection: settings,
        aspeech: false,
    };
    let mut recorder = Recorder::default();
    let sending = audio_speech(call, &mut recorder, &transport).unwrap();
    let (lines, reply) = tokio::join!(serve(listener, status), sending);
    let response = reply.expect("the redirect is followed to the audio");
    assert_eq!(response.status, 200);
    assert_eq!(response.content(16).await.unwrap(), vec![7, 8, 9]);
    assert_eq!(
        lines,
        vec![
            String::from("POST /v1/audio/speech HTTP/1.1"),
            second_line.to_owned()
        ]
    );
}

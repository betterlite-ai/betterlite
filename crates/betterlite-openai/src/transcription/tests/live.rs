use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use indexmap::IndexMap;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;

use super::cases::{BOUNDARY, default_connection};
use super::fixture::{AudioFixture, Recorder};
use crate::api::audio_file_input::get_audio_file_name;
use crate::transcription::call::{
    TranscriptionCall, TranscriptionRequest, audio_transcriptions, finish_transcription,
};
use crate::transcription::config::TranscriptionFamily;
use crate::transcription::transport::TranscriptionTransport;

async fn serve_transcription(listener: TcpListener) -> String {
    let (mut stream, _) = listener.accept().await.expect("the client connects");
    let mut received = Vec::new();
    let mut chunk = [0_u8; 1024];
    let last = format!("--{BOUNDARY}--");
    while !received
        .windows(last.len())
        .any(|window| window == last.as_bytes())
    {
        let read = stream.read(&mut chunk).await.expect("the request arrives");
        received.extend_from_slice(&chunk[..read]);
    }
    let reply = "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 13\r\nconnection: close\r\n\r\n{\"text\":\"hi\"}";
    stream
        .write_all(reply.as_bytes())
        .await
        .expect("the reply is written");
    String::from_utf8(received).expect("the request is text")
}

#[tokio::test]
async fn proxied_client_sends_the_transcription() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("the listener binds");
    let address = listener.local_addr().expect("the listener has an address");
    let audio = AudioFixture::new("a.wav", b"abc", "audio/wav");
    let processed = audio.processed();
    let mut connection = default_connection();
    connection.api_base = Some(format!("http://{address}/v1"));
    let prepared = TranscriptionCall {
        input: get_audio_file_name(&audio.audio_file()),
        file: &processed,
        connection,
        boundary: BOUNDARY,
    };
    let request = TranscriptionRequest {
        model: "whisper-1",
        family: Some(TranscriptionFamily::Whisper),
        optional_params: IndexMap::new(),
        atranscription: false,
    };
    let (outbound, pending) = audio_transcriptions(&prepared, &request, &mut Recorder::default())
        .expect("the call begins");
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .expect("the policy resolves"),
    );
    let client = ProxiedClient::outbound(&policy).expect("the client builds");
    let (head, sent) = tokio::join!(
        serve_transcription(listener),
        TranscriptionTransport::send(&client, outbound)
    );
    assert!(
        head.starts_with("POST /v1/audio/transcriptions HTTP/1.1"),
        "{head}"
    );
    assert!(head.contains("filename=\"a.wav\""), "{head}");
    let reply = finish_transcription(pending, sent, &mut Recorder::default())
        .expect("the 200 reply is a success");
    assert_eq!(reply.response.text.as_deref(), Some("hi"));
}

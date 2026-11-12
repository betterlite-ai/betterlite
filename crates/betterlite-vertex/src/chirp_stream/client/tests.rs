#![cfg(test)]

use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

use google_cloud_auth::credentials::CacheableResource;
use google_cloud_gax::Result as GaxResult;
use google_cloud_gax::options::RequestOptions;
use google_cloud_gax::streaming::{RequestSender, ResponseStream};
use google_cloud_speech_v2::client::Speech;
use google_cloud_speech_v2::model::{
    StreamingRecognizeRequest, StreamingRecognizeResponse,
    streaming_recognize_response::SpeechEventType,
};
use google_cloud_speech_v2::stub::Speech as StubSpeech;
use http::header::AUTHORIZATION;
use rstest::rstest;
use secrecy::SecretString;
use tokio::sync::mpsc;

use super::{
    RequestSink as _, ResponseSource as _, SdkRequestSink, SdkResponseSource, SdkSpeechClient,
    SpeechStreamingClient as _, bearer_headers, speech_endpoint,
};
use crate::chirp_stream::error::ChirpStreamError;

#[test]
fn bearer_headers_sends_a_sensitive_authorization_header() {
    let headers = bearer_headers(&SecretString::from("token-value".to_owned())).expect("a header");
    let CacheableResource::New { data, .. } = headers else {
        panic!("the token header is always new");
    };
    let header = data.get(AUTHORIZATION).expect("an authorization header");
    assert_eq!(header.to_str().ok(), Some("Bearer token-value"));
    assert!(header.is_sensitive(), "the token must not print in debug");
}

#[rstest]
#[case::host_only(
    "us-central1-speech.googleapis.com",
    "https://us-central1-speech.googleapis.com"
)]
#[case::with_scheme(
    "https://speech.googleapis.com:443",
    "https://speech.googleapis.com:443"
)]
fn a_host_only_endpoint_dials_https(#[case] given: &str, #[case] expected: &str) {
    assert_eq!(speech_endpoint(given), expected);
}

#[tokio::test]
async fn sdk_request_sink_forwards_the_request() {
    let (sender, mut receiver) = mpsc::channel::<StreamingRecognizeRequest>(1);
    let mut sink = SdkRequestSink {
        sender: RequestSender::from(sender),
    };
    let request = StreamingRecognizeRequest::new().set_recognizer("projects/p/recognizers/_");
    sink.send(request.clone()).await.expect("the send succeeds");
    assert_eq!(receiver.recv().await, Some(request));
}

#[tokio::test]
async fn sdk_response_source_reads_the_stream() {
    let (sender, receiver) = mpsc::channel::<GaxResult<StreamingRecognizeResponse>>(1);
    let mut source = SdkResponseSource {
        responses: ResponseStream::from(receiver),
    };
    let response = StreamingRecognizeResponse::new()
        .set_speech_event_type(SpeechEventType::SpeechActivityBegin);
    sender
        .send(Ok(response.clone()))
        .await
        .expect("the send succeeds");
    match source.next_response().await {
        Some(Ok(incoming)) => assert_eq!(incoming, response),
        Some(Err(error)) => panic!("unexpected stream error: {error}"),
        None => panic!("the response was not received"),
    }
}

#[test]
fn close_drops_the_speech_client_once() {
    let client = SdkSpeechClient {
        speech: Mutex::new(Some(Speech::from_stub(StubStream))),
        closed: AtomicBool::new(false),
    };
    let _stream = client
        .streaming_recognize()
        .expect("the stream opens before close");
    client.close();
    assert!(matches!(
        client.streaming_recognize(),
        Err(ChirpStreamError::ClientClosed)
    ));
    client.close();
    assert!(matches!(
        client.streaming_recognize(),
        Err(ChirpStreamError::ClientClosed)
    ));
}

#[derive(Debug)]
struct StubStream;

impl StubSpeech for StubStream {
    fn streaming_recognize(
        &self,
        _options: RequestOptions,
    ) -> (
        RequestSender<StreamingRecognizeRequest>,
        ResponseStream<StreamingRecognizeResponse>,
    ) {
        let (request_sender, _requests) = mpsc::channel::<StreamingRecognizeRequest>(1);
        let (_responses, response_receiver) =
            mpsc::channel::<GaxResult<StreamingRecognizeResponse>>(1);
        (
            RequestSender::from(request_sender),
            ResponseStream::from(response_receiver),
        )
    }
}

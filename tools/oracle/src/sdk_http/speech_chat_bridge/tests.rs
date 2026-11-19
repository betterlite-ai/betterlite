#![cfg(test)]

use std::collections::BTreeMap;
use std::path::Path;

use crate::sdk_http::OracleLimits;
use crate::sdk_http::call_kind::CallKind as _;
use crate::sdk_http::calls::SdkHttpCall;
use crate::sdk_http::decode_sdk_http_case;
use crate::sdk_http::result::Failure;
use crate::sdk_http::result::kinds::SdkResult;

// an unroutable port proves the rejection stops before any provider HTTP
#[tokio::test]
async fn replays_the_rejected_speech_bridge_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fuzz/corpus/sdk_http_case/speech-chat-bridge-rejection.yaml");
    let source = std::fs::read_to_string(corpus).expect("the rejection corpus");
    let case = decode_sdk_http_case(source.as_bytes(), &OracleLimits::default())
        .expect("the rejection corpus decodes");
    let SdkHttpCall::SpeechChatBridge(call) = case.call else {
        panic!("the rejection corpus must be a speech bridge call");
    };
    let outcome = call
        .send("http://127.0.0.1:9", &BTreeMap::new())
        .await
        .expect("the rejection needs no provider");
    assert_eq!(
        outcome,
        SdkResult::Failure(Failure {
            class: String::from("BadRequestError"),
            status: 400,
            kind: Some(None),
            param: Some(None),
            code: Some(None),
            message: String::from(
                "litellm.BadRequestError: Gemini TTS only produces raw PCM16 audio, so response_format='mp3' is not supported. Supported response formats: pcm, wav."
            ),
        })
    );
}

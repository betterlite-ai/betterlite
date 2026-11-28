use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_bedrock::transcription::binding::TranscriptionCall;
use betterlite_bedrock::transcription::params::Params;
use betterlite_openai::batches::sdk::{Timeout, TimeoutPhases};

use super::fixture::{Fixture, Handler, run_transcription, transcription_params, value};
use crate::bedrock_other_dispatch::call::{
    BedrockOtherOperation, TranscriptionFile, TranscriptionOperation,
};
use crate::bedrock_other_dispatch::error::BedrockOtherError;
use crate::bedrock_other_dispatch::test_support::binding::Recording;
use crate::bedrock_other_dispatch::test_support::client::Reply;
use crate::bedrock_other_dispatch::test_support::{credentials, native_binding};

const ACCESS_KEY: &str = "AKIAIOSFODNN7EXAMPLE";
const SECRET_KEY: &str = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";
const AUDIO: &[u8] = b"hi";
const REPLY: &str = r#"{"output": {"message": {"content": [{"text": "hello world"}]}}}"#;

fn scripted(replies: &[Reply]) -> Fixture {
    Fixture::new(replies, credentials(ACCESS_KEY, SECRET_KEY))
}

// The arguments one transcription case hands the dispatcher
struct Call<'call> {
    model: &'call str,
    file: TranscriptionFile<'call>,
    optional: &'call Params,
    timeout: Option<&'call Timeout>,
    asynchronous: bool,
}

fn operation<B>(call: Call<'_>, binding: B) -> BedrockOtherOperation<'_, '_, '_, Handler, B> {
    BedrockOtherOperation::Transcription(TranscriptionOperation {
        call: TranscriptionCall {
            model: call.model,
            api_key: None,
            api_base: None,
            custom_llm_provider: "bedrock",
            extra_headers: None,
            optional_params: call.optional,
            timeout: call.timeout,
        },
        file: call.file,
        asynchronous: call.asynchronous,
        binding,
    })
}

fn native(fixture: &Fixture, environment: BTreeMap<String, String>) -> super::fixture::Binding {
    native_binding(fixture.client(), fixture.source().clone(), environment)
}

// One seam row: the typed file, its timeout, the seam, the answer text, the optional params and the expected arguments
struct Seam {
    file: TranscriptionFile<'static>,
    timeout: Option<Timeout>,
    asynchronous: bool,
    text: &'static str,
    optional: &'static str,
    expected: &'static str,
}

fn seam_cases() -> [Seam; 2] {
    [
        Seam {
            file: TranscriptionFile::Bytes(AUDIO),
            timeout: Some(Timeout::Seconds(2.5)),
            asynchronous: false,
            text: "transcribed",
            optional: r#"{"language": "en"}"#,
            expected: r#"{"model": "amazon.nova-2-sonic-v1:0", "audio": {"data": "aGk=", "format": "wav", "filename": "audio.wav"}, "api_key": null, "api_base": null, "custom_llm_provider": "bedrock", "extra_headers": null, "optional_params": {"language": "en"}, "timeout_seconds": 2.5}"#,
        },
        Seam {
            file: TranscriptionFile::Multipart {
                filename: "voice.mp3",
                content: b"ID3".as_slice(),
            },
            timeout: Some(Timeout::Phases(TimeoutPhases {
                connect: Some(1.0_f64),
                read: Some(600.0_f64),
                write: Some(30.0_f64),
                pool: Some(5.0_f64),
            })),
            asynchronous: true,
            text: "tuple text",
            optional: "{}",
            expected: r#"{"model": "amazon.nova-2-sonic-v1:0", "audio": {"data": "SUQz", "format": "mp3", "filename": "voice.mp3"}, "api_key": null, "api_base": null, "custom_llm_provider": "bedrock", "extra_headers": null, "optional_params": {}, "timeout_seconds": 600.0}"#,
        },
    ]
}

// Both seams: bytes reach the NO_PYTHON binding as wav, a typed multipart file keeps its filename, and a phased timeout keeps its read phase
async fn file_seams() {
    for seam in seam_cases() {
        assert_file_seam(seam).await;
    }
}

async fn assert_file_seam(seam: Seam) {
    let fixture = scripted(&[]);
    let params = transcription_params(seam.optional);
    let recording = Recording::new(seam.text);
    let observer = recording.clone();
    let outcome = run_transcription(
        &fixture,
        operation(
            Call {
                model: "amazon.nova-2-sonic-v1:0",
                file: seam.file,
                optional: &params,
                timeout: seam.timeout.as_ref(),
                asynchronous: seam.asynchronous,
            },
            recording,
        ),
    )
    .await;
    assert_eq!(outcome.transcription().text.as_deref(), Some(seam.text));
    assert_eq!(observer.seen.values(), vec![value(seam.expected)]);
    assert!(outcome.requests.is_empty());
}

// The native binding sends the request its own url and model policy name and decodes the reply
async fn native_signed_round_trip() {
    let fixture = scripted(&[Reply::status(200, REPLY)]);
    let optional = transcription_params(r#"{"language": "en"}"#);
    let timeout = Timeout::Seconds(2.5);
    let outcome = run_transcription(
        &fixture,
        operation(
            Call {
                model: "bedrock/converse/eu-west-1/nova/transcribe-model",
                file: TranscriptionFile::Bytes(AUDIO),
                optional: &optional,
                timeout: Some(&timeout),
                asynchronous: true,
            },
            native(&fixture, BTreeMap::new()),
        ),
    )
    .await;
    assert_eq!(outcome.requests.len(), 1);
    assert_eq!(outcome.posted().method, "POST");
    assert_eq!(
        outcome.posted().url,
        "https://bedrock-runtime.eu-west-1.amazonaws.com/model/transcribe-model/converse"
    );
    assert_eq!(outcome.posted().deadline, Some(Duration::from_millis(2500)));
    assert!(
        outcome
            .authorization()
            .is_some_and(|header| header.starts_with("AWS4-HMAC-SHA256"))
    );
    assert_eq!(
        outcome.body(0),
        value(
            r#"{"messages": [{"role": "user", "content": [{"audio": {"format": "wav", "source": {"bytes": "aGk="}}}, {"text": "Transcribe the audio. Respond with only the transcript. The audio language is en."}]}], "system": [{"text": "You are a transcription assistant."}], "inferenceConfig": {"maxTokens": 4096}}"#
        )
    );
    assert_eq!(outcome.transcription().text.as_deref(), Some("hello world"));
}

// An explicit aws_region_name beats the environment, and the default deadline is ten minutes
async fn native_region_override() {
    let fixture = scripted(&[Reply::status(200, REPLY)]);
    let explicit =
        transcription_params(r#"{"aws_region_name": "ap-south-1", "prompt": "a meeting"}"#);
    let environment = BTreeMap::from([(String::from("AWS_REGION"), String::from("us-west-2"))]);
    let outcome = run_transcription(
        &fixture,
        operation(
            Call {
                model: "plain-model",
                file: TranscriptionFile::Bytes(AUDIO),
                optional: &explicit,
                timeout: None,
                asynchronous: true,
            },
            native(&fixture, environment),
        ),
    )
    .await;
    assert_eq!(
        outcome.posted().url,
        "https://bedrock-runtime.ap-south-1.amazonaws.com/model/plain-model/converse"
    );
    assert_eq!(outcome.posted().deadline, Some(Duration::from_mins(10)));
    assert_eq!(
        outcome.body(0),
        value(
            r#"{"messages": [{"role": "user", "content": [{"audio": {"format": "wav", "source": {"bytes": "aGk="}}}, {"text": "Transcribe the audio. Respond with only the transcript. Additional context: a meeting"}]}], "system": [{"text": "You are a transcription assistant."}], "inferenceConfig": {"maxTokens": 4096}}"#
        )
    );
}

// A synchronous native call is refused by the leaf before any request goes out
async fn blocking_native_call() {
    let fixture = scripted(&[]);
    let empty = transcription_params("{}");
    let blocking = run_transcription(
        &fixture,
        operation(
            Call {
                model: "plain-model",
                file: TranscriptionFile::Bytes(AUDIO),
                optional: &empty,
                timeout: None,
                asynchronous: false,
            },
            native(&fixture, BTreeMap::new()),
        ),
    )
    .await;
    assert!(matches!(blocking.error(), BedrockOtherError::Binding(_)));
    assert!(blocking.requests.is_empty());
}

// An unsupported audio container is refused by name, with no request and no status
async fn unsupported_file() {
    let fixture = scripted(&[]);
    let empty = transcription_params("{}");
    let text_file = TranscriptionFile::Multipart {
        filename: "voice.txt",
        content: AUDIO,
    };
    let unsupported = run_transcription(
        &fixture,
        operation(
            Call {
                model: "plain-model",
                file: text_file,
                optional: &empty,
                timeout: None,
                asynchronous: true,
            },
            native(&fixture, BTreeMap::new()),
        ),
    )
    .await;
    assert_eq!(
        unsupported.error().to_string(),
        "Unsupported Bedrock audio format for file 'voice.txt'"
    );
    assert_eq!(unsupported.error().status(), None);
    assert!(unsupported.requests.is_empty());
}

// A binding failure travels as the composed error's binding arm with no status of its own
async fn failed_binding() {
    let fixture = scripted(&[]);
    let empty = transcription_params("{}");
    let failed = run_transcription(
        &fixture,
        operation(
            Call {
                model: "plain-model",
                file: TranscriptionFile::Bytes(AUDIO),
                optional: &empty,
                timeout: None,
                asynchronous: false,
            },
            Recording::failing(500),
        ),
    )
    .await;
    assert!(matches!(failed.error(), BedrockOtherError::Binding(_)));
    assert_eq!(failed.error().status(), None);
}

pub(super) async fn case() {
    file_seams().await;
    native_signed_round_trip().await;
    native_region_override().await;
    blocking_native_call().await;
    unsupported_file().await;
    failed_binding().await;
}

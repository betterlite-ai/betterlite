use betterlite_openai::api::audio_file_input::{
    AudioFile, AudioFileError, AudioTuple, TupleContent,
};
use betterlite_openai::batches::sdk::{Timeout, TimeoutPhases};
use rstest::rstest;

use super::support::{NoFiles, params};
use crate::transcription::audio::{AudioFormat, AudioPayload, audio_format, audio_payload};
use crate::transcription::binding::{
    BindingArguments, BindingError, TranscriptionBinding, TranscriptionCall,
    async_audio_transcriptions, audio_transcriptions, prepare_arguments,
};
use crate::transcription::error::TranscriptionError;
use crate::transcription::params::Params;

fn tuple<'file>(filename: &'file str, bytes: &'file [u8]) -> AudioFile<'file> {
    AudioFile::Tuple(AudioTuple {
        filename,
        content: Some(TupleContent::Bytes(bytes)),
    })
}

fn formats(rows: &[(&str, &str, Option<AudioFormat>)]) {
    for (content_type, filename, expected) in rows {
        assert_eq!(
            audio_format(content_type, filename),
            *expected,
            "{content_type} {filename}"
        );
    }
}

fn wav_mime_payload() {
    formats(&[
        ("audio/wav", "x.bin", Some(AudioFormat::Wav)),
        ("audio/x-wav", "x.bin", Some(AudioFormat::Wav)),
        ("audio/flac", "x.bin", Some(AudioFormat::Flac)),
        ("audio/ogg", "x.bin", Some(AudioFormat::Ogg)),
        ("audio/WAV", "x.bin", None),
        ("audio/WAV", "x.mp3", Some(AudioFormat::Mp3)),
    ]);
    let payload = audio_payload(AudioFile::Bytes(b"RIFF"), &NoFiles).unwrap();
    assert_eq!(
        payload,
        AudioPayload {
            data: "UklGRg==".to_owned(),
            format: AudioFormat::Wav,
            filename: "audio.wav".to_owned(),
        }
    );
    let standard = audio_payload(AudioFile::Bytes(&[0xfb, 0xff, 0xbf]), &NoFiles).unwrap();
    assert_eq!(standard.data, "+/+/");
    let empty = audio_payload(AudioFile::ByteArray(b""), &NoFiles).unwrap();
    assert_eq!(empty.data, "");
    let encoded = serde_json::to_string(&payload).unwrap();
    assert_eq!(
        encoded,
        r#"{"data":"UklGRg==","format":"wav","filename":"audio.wav"}"#
    );
}

fn mp3_mime_alias() {
    formats(&[
        ("audio/mpeg", "x.wav", Some(AudioFormat::Mp3)),
        ("audio/mp3", "x.flac", Some(AudioFormat::Mp3)),
        ("audio/mpeg", "x", Some(AudioFormat::Mp3)),
    ]);
    let payload = audio_payload(tuple("My Clip.mp3", b"ID3"), &NoFiles).unwrap();
    assert_eq!(payload.format, AudioFormat::Mp3);
    assert_eq!(payload.filename, "My Clip.mp3");
    assert_eq!(payload.data, "SUQz");
}

fn filename_format_fallback() {
    formats(&[
        ("application/octet-stream", "a.WAV", Some(AudioFormat::Wav)),
        ("x", "a.b.Mp3", Some(AudioFormat::Mp3)),
        ("x", "a.flac", Some(AudioFormat::Flac)),
        ("x", "a.ogg", Some(AudioFormat::Ogg)),
        ("x", ".wav", Some(AudioFormat::Wav)),
        ("x", "a.", None),
        ("x", "wav", None),
        ("x", "", None),
        ("x", "a.opus", None),
        ("x", "a.wav.exe", None),
    ]);
    let payload = audio_payload(tuple("a.xyz", b"RIFF"), &NoFiles).unwrap();
    assert_eq!(payload.format, AudioFormat::Wav);
    assert_eq!(payload.filename, "a.xyz");
}

fn unsupported_audio_format() {
    let failure = audio_payload(tuple("a.aac", b"x"), &NoFiles).unwrap_err();
    assert!(matches!(
        &failure,
        TranscriptionError::UnsupportedFormat { filename } if filename == "a.aac"
    ));
    assert_eq!(
        failure.to_string(),
        "Unsupported Bedrock audio format for file 'a.aac'"
    );
    let bare = audio_payload(AudioFile::Text("a.wav"), &NoFiles).unwrap_err();
    assert!(matches!(
        bare,
        TranscriptionError::AudioFile(AudioFileError::BareString)
    ));
}

struct Fake {
    reply: Option<Params>,
}

#[derive(Debug, thiserror::Error)]
#[error("binding down")]
struct Down;

#[async_trait::async_trait]
impl TranscriptionBinding for Fake {
    type Error = Down;

    fn transcribe(&self, _arguments: &BindingArguments<'_>) -> Result<Params, Down> {
        self.reply.clone().ok_or(Down)
    }

    async fn atranscribe(&self, _arguments: &BindingArguments<'_>) -> Result<Params, Down> {
        self.reply.clone().ok_or(Down)
    }
}

type Outcome = Result<Option<String>, &'static str>;

fn label(error: BindingError<Down>) -> &'static str {
    match error {
        BindingError::Binding(Down) => "binding",
        BindingError::Response(_) => "response",
    }
}

fn both_paths(reply: Option<&str>, arguments: &BindingArguments<'_>) -> [Outcome; 2] {
    let binding = Fake {
        reply: reply.map(params),
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let asynchronous = runtime.block_on(async_audio_transcriptions(&binding, arguments));
    [audio_transcriptions(&binding, arguments), asynchronous]
        .map(|result| result.map(|response| response.text).map_err(label))
}

fn phases(read: Option<f64>) -> Timeout {
    Timeout::Phases(TimeoutPhases {
        connect: Some(1.0),
        read,
        write: Some(3.0),
        pool: Some(4.0),
    })
}

fn call<'call>(
    timeout: Option<&'call Timeout>,
    optional_params: &'call Params,
) -> TranscriptionCall<'call> {
    TranscriptionCall {
        model: "bedrock/voxtral",
        api_key: None,
        api_base: Some("https://base"),
        custom_llm_provider: "bedrock",
        extra_headers: None,
        optional_params,
        timeout,
    }
}

fn binding_replies() {
    let options = params(r#"{"language":"en"}"#);
    let source = call(None, &options);
    let arguments = prepare_arguments(tuple("a.wav", b"x"), &NoFiles, &source).unwrap();
    let hello = Ok(Some("hello".to_owned()));
    assert_eq!(
        both_paths(Some(r#"{"text":"hello"}"#), &arguments),
        [hello.clone(), hello]
    );
    assert_eq!(both_paths(Some("{}"), &arguments), [Ok(None), Ok(None)]);
    assert_eq!(
        both_paths(Some(r#"{"text":null}"#), &arguments),
        [Ok(None), Ok(None)]
    );
    for reply in [r#"{"text":"x","usage":1}"#, r#"{"text":5}"#] {
        let rejected = both_paths(Some(reply), &arguments);
        assert_eq!(rejected, [Err("response"), Err("response")], "{reply}");
    }
    assert_eq!(
        both_paths(None, &arguments),
        [Err("binding"), Err("binding")]
    );
}

fn transcription_binding_arguments() {
    let options = params(r#"{"language":"en"}"#);
    let timeouts = [
        (None, None),
        (Some(Timeout::Seconds(3.5)), Some(3.5)),
        (Some(Timeout::Seconds(0.0)), Some(0.0)),
        (Some(phases(Some(2.0))), Some(2.0)),
        (Some(phases(None)), None),
    ];
    for (timeout, seconds) in &timeouts {
        let source = call(timeout.as_ref(), &options);
        let arguments = prepare_arguments(tuple("a.ogg", b"OggS"), &NoFiles, &source).unwrap();
        assert_eq!(arguments.timeout_seconds, *seconds, "{timeout:?}");
        assert_eq!(arguments.audio.format, AudioFormat::Ogg);
        assert_eq!(arguments.call.model, "bedrock/voxtral");
        assert_eq!(arguments.call.api_base, Some("https://base"));
        assert_eq!(arguments.call.optional_params, &options);
    }
    let source = call(None, &options);
    let rejected = prepare_arguments(tuple("a.aac", b"x"), &NoFiles, &source);
    assert!(matches!(
        rejected,
        Err(TranscriptionError::UnsupportedFormat { .. })
    ));
    binding_replies();
}

#[rstest]
#[case::wav_mime_payload(wav_mime_payload)]
#[case::mp3_mime_alias(mp3_mime_alias)]
#[case::filename_format_fallback(filename_format_fallback)]
#[case::unsupported_audio_format(unsupported_audio_format)]
#[case::transcription_binding_arguments(transcription_binding_arguments)]
fn transcription_cases(#[case] check: fn()) {
    check();
}

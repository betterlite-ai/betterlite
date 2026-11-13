use std::fs;
use std::path::{Path, PathBuf};

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use super::super::decode::{ReplyFormat, decode_transcription_reply, reply_format};
use crate::transcription::error::TranscriptionError;

fn category(path: &Path) -> String {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("the seed name is UTF-8");
    name.split('-')
        .next()
        .expect("the seed has a category")
        .to_owned()
}

fn replays(path: &Path) -> bool {
    let bytes = fs::read(path).expect("the seed reads");
    let json =
        decode_transcription_reply(&bytes, Some("application/json"), ReplyFormat::Transcription);
    let text = decode_transcription_reply(&bytes, Some("text/plain"), ReplyFormat::Text);
    match category(path).as_str() {
        "valid" => json.is_ok(),
        "nonobject" => matches!(json, Err(TranscriptionError::NotObject)),
        "malformed" | "truncated" => matches!(json, Err(TranscriptionError::Json { .. })),
        "text" => text.is_ok() && matches!(json, Err(TranscriptionError::Json { .. })),
        // a boundary seed only has to come back; its JSON half may be either outcome
        "boundary" => text.is_ok(),
        other => panic!("unknown corpus category {other}"),
    }
}

#[test]
fn replays_the_openai_transcription_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/openai_transcription");
    let seeds: Vec<PathBuf> = fs::read_dir(&corpus)
        .expect("the corpus folder exists")
        .map(|entry| entry.expect("the corpus entry reads").path())
        .collect();
    assert!(!seeds.is_empty(), "the corpus has seeds");
    assert_eq!(
        seeds
            .iter()
            .find(|path| !replays(path))
            .map(PathBuf::as_path),
        None
    );
}

// what the OpenAI SDK's model_dump gave for each body, read off openai 2.54.0
#[rstest]
#[case::verbose_segments(
    ReplyFormat::Verbose,
    r#"{"task":"transcribe","language":"en","duration":1.5,"text":"hi","words":[{"word":"hi","start":0.0,"end":1.0}],"segments":[{"id":0,"start":0.0,"end":1.5,"text":"hi"}],"usage":{"type":"duration","seconds":1.5}}"#,
    r#"{"duration":1.5,"language":"en","text":"hi","segments":[{"id":0,"avg_logprob":null,"compression_ratio":null,"end":1.5,"no_speech_prob":null,"seek":null,"start":0.0,"temperature":null,"text":"hi","tokens":null}],"usage":{"seconds":1.5,"type":"duration"},"words":[{"end":1.0,"start":0.0,"word":"hi"}],"task":"transcribe"}"#
)]
#[case::verbose_usage_is_duration(
    ReplyFormat::Verbose,
    r#"{"text":"hi","usage":{"type":"tokens"}}"#,
    r#"{"duration":null,"language":null,"text":"hi","segments":null,"usage":{"seconds":null,"type":"tokens"},"words":null}"#
)]
#[case::transcription_tokens(
    ReplyFormat::Transcription,
    r#"{"text":"hi","usage":{"type":"tokens","input_tokens":1,"input_token_details":{"audio_tokens":1}},"logprobs":[{"token":"a"}],"languages":[{"code":"en"},3]}"#,
    r#"{"text":"hi","languages":[{"code":"en"},3],"logprobs":[{"token":"a","bytes":null,"logprob":null}],"usage":{"input_tokens":1,"output_tokens":null,"total_tokens":null,"type":"tokens","input_token_details":{"audio_tokens":1,"text_tokens":null}}}"#
)]
#[case::transcription_other_usage(
    ReplyFormat::Transcription,
    r#"{"text":"hi","usage":{"type":"weird","x":1},"logprobs":"no"}"#,
    r#"{"text":"hi","languages":null,"logprobs":"no","usage":{"input_tokens":null,"output_tokens":null,"total_tokens":null,"type":"weird","input_token_details":null,"x":1}}"#
)]
#[case::transcription_duration(
    ReplyFormat::Transcription,
    r#"{"text":"hi","usage":{"type":"duration","seconds":1.5}}"#,
    r#"{"text":"hi","languages":null,"logprobs":null,"usage":{"seconds":1.5,"type":"duration"}}"#
)]
#[case::diarized(
    ReplyFormat::Diarized,
    r#"{"text":"hi","usage":{"type":"tokens"},"segments":[{"id":"s"}]}"#,
    r#"{"duration":null,"segments":[{"id":"s","end":null,"speaker":null,"start":null,"text":null,"type":null}],"task":null,"text":"hi","usage":{"input_tokens":null,"output_tokens":null,"total_tokens":null,"type":"tokens","input_token_details":null}}"#
)]
fn dumps_the_reply_like_the_sdk_model(
    #[case] format: ReplyFormat,
    #[case] body: &str,
    #[case] dumped: &str,
) {
    let reply = decode_transcription_reply(body.as_bytes(), Some("application/json"), format)
        .expect("the body decodes");
    let expected = serde_json::from_str::<Json>(dumped).expect("the dump is JSON");
    let (Json::Object(fields), Json::Object(expected_fields)) =
        (&reply.original_response, &expected)
    else {
        panic!("both dumps are objects: {:?}", reply.original_response);
    };
    assert_eq!(
        fields.keys().collect::<Vec<_>>(),
        expected_fields.keys().collect::<Vec<_>>()
    );
    assert_eq!(reply.original_response, expected);
}

#[rstest]
#[case::absent(None, ReplyFormat::Transcription)]
#[case::json(Some("json"), ReplyFormat::Transcription)]
#[case::verbose(Some("verbose_json"), ReplyFormat::Verbose)]
#[case::diarized(Some("diarized_json"), ReplyFormat::Diarized)]
#[case::srt(Some("srt"), ReplyFormat::Text)]
fn picks_the_sdk_model_from_response_format(
    #[case] response_format: Option<&str>,
    #[case] expected: ReplyFormat,
) {
    let options = response_format.map_or_else(IndexMap::new, |name| {
        IndexMap::from([(String::from("response_format"), Json::Text(name.to_owned()))])
    });
    assert_eq!(reply_format(&options).expect("a string format"), expected);
}

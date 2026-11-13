use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::cases::{SyncCall, default_connection, list, number, part_values, run, text, wire};
use super::fixture::{AudioFixture, Fixture, Reply};
use crate::transcription::config::TranscriptionFamily;
use crate::transcription::error::TranscriptionError;
use crate::transcription::request::prepare_data;

async fn sent_parts(
    family: Option<TranscriptionFamily>,
    options: IndexMap<String, Json>,
) -> Result<Vec<MultipartPart>, TranscriptionError> {
    let audio = AudioFixture::new("a.wav", b"abc", "audio/wav");
    let fixture = Fixture::new(Reply::json(r#"{"text":"hi"}"#));
    let call = SyncCall {
        model: "whisper-1",
        family,
        options,
        data: None,
        connection: default_connection(),
    };
    run(&fixture, &audio, call)
        .await
        .outcome
        .map(|_reply| wire(&fixture).parts)
}

fn part_names(parts: &[MultipartPart]) -> Vec<&str> {
    parts.iter().map(|part| part.name.as_str()).collect()
}

fn object(fields: &[(&str, Json)]) -> Json {
    Json::Object(
        fields
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect(),
    )
}

fn named(fields: &[(&str, Json)]) -> IndexMap<String, Json> {
    fields
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

fn every_sdk_option() -> IndexMap<String, Json> {
    named(&[
        ("timestamp_granularities", list(&["word"])),
        ("temperature", number("0.2")),
        ("stream", Json::Bool(false)),
        ("response_format", text("json")),
        ("prompt", text("names")),
        ("languages", list(&["en", "de"])),
        ("language", text("en")),
        (
            "known_speaker_references",
            list(&["data:audio/wav;base64,AAA"]),
        ),
        ("known_speaker_names", list(&["alice"])),
        ("keywords", list(&["acme"])),
        ("include", list(&["logprobs"])),
        (
            "chunking_strategy",
            object(&[("type", text("server_vad")), ("threshold", number("0.5"))]),
        ),
    ])
}

// every parameter of the SDK's create(), sent in the signature's order after the model and before the file
#[tokio::test]
async fn sends_every_sdk_transcription_field() {
    let options = every_sdk_option();
    let parts = sent_parts(None, options)
        .await
        .expect("every field is sent");
    assert_eq!(
        part_names(&parts),
        [
            "model",
            "chunking_strategy[type]",
            "chunking_strategy[threshold]",
            "include[]",
            "keywords[]",
            "known_speaker_names[]",
            "known_speaker_references[]",
            "language",
            "languages[]",
            "languages[]",
            "prompt",
            "response_format",
            "stream",
            "temperature",
            "timestamp_granularities[]",
            "file"
        ]
    );
    assert_eq!(part_values(&parts, "stream"), ["false"]);
    assert_eq!(part_values(&parts, "temperature"), ["0.2"]);
    assert_eq!(part_values(&parts, "languages[]"), ["en", "de"]);
    assert_eq!(part_values(&parts, "chunking_strategy[threshold]"), ["0.5"]);
}

// the SDK's own scalar rules: a flag is true or false, null and an empty text send nothing
#[tokio::test]
async fn encodes_transcription_scalars_like_the_sdk() {
    let flag = sent_parts(None, named(&[("stream", Json::Bool(true))]))
        .await
        .expect("a boolean is sent");
    assert_eq!(part_values(&flag, "stream"), ["true"]);
    let options = named(&[
        ("language", Json::Null),
        ("prompt", text("")),
        ("include", Json::List(vec![Json::Null, text("logprobs")])),
        ("keywords", Json::List(vec![text("")])),
    ]);
    let dropped = sent_parts(None, options).await.expect("nothing to send");
    assert_eq!(part_names(&dropped), ["model", "include[]", "file"]);
    assert_eq!(part_values(&dropped, "include[]"), ["logprobs"]);
}

// the allowlist does not follow the family: a parameter map_openai_params would drop is still sent
#[tokio::test]
async fn sends_params_the_family_does_not_list() {
    let whisper = sent_parts(
        Some(TranscriptionFamily::Whisper),
        named(&[("include", list(&["logprobs"]))]),
    )
    .await
    .expect("whisper sends include");
    assert_eq!(part_values(&whisper, "include[]"), ["logprobs"]);
    let gpt = sent_parts(
        Some(TranscriptionFamily::Gpt),
        named(&[("timestamp_granularities", list(&["word"]))]),
    )
    .await
    .expect("gpt sends timestamp_granularities");
    assert_eq!(part_values(&gpt, "timestamp_granularities[]"), ["word"]);
}

// what the SDK's create() has no keyword for, and the audio file the call already holds
#[tokio::test]
async fn refuses_fields_the_sdk_cannot_send() {
    let refused = sent_parts(None, named(&[("bogus", text("x"))]))
        .await
        .expect_err("the SDK has no bogus parameter");
    assert!(
        matches!(&refused, TranscriptionError::Field { field, problem } if field == "bogus" && *problem == "is not one the OpenAI SDK sends"),
        "{refused:?}"
    );
    let file = prepare_data(
        None,
        "whisper-1",
        &IndexMap::from([(String::from("file"), text("x"))]),
    )
    .expect_err("the file is the audio file");
    assert!(matches!(&file, TranscriptionError::Field { field, .. } if field == "file"));
}

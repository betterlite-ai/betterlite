use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::cases::{SyncCall, default_connection, list, number, part_values, run, text, wire};
use super::fixture::{AudioFixture, Fixture, Reply};
use crate::transcription::error::TranscriptionError;
use crate::transcription::request::prepare_data;

async fn field_outcome(name: &str, value: Json) -> Result<Vec<MultipartPart>, TranscriptionError> {
    let audio = AudioFixture::new("a.wav", b"abc", "audio/wav");
    let fixture = Fixture::new(Reply::json(r#"{"text":"hi"}"#));
    let call = SyncCall {
        model: "whisper-1",
        family: None,
        options: IndexMap::from([(name.to_owned(), value)]),
        data: None,
        connection: default_connection(),
    };
    run(&fixture, &audio, call)
        .await
        .outcome
        .map(|_reply| wire(&fixture).parts)
}

async fn assert_field_refused(name: &str, value: Json, problem: &str) {
    let refused = field_outcome(name, value)
        .await
        .expect_err("the SDK cannot send this value");
    assert!(
        matches!(&refused, TranscriptionError::Field { field, problem: found } if field == name && *found == problem),
        "{name}: {refused:?}"
    );
}

// every multipart field kind: scalars, the two list fields, null, and what the SDK cannot encode
#[tokio::test]
async fn rejects_malformed_transcription_fields() {
    let listed = "is not a list of strings";
    let scalar = "is not a scalar";
    assert_field_refused("include", text("logprobs"), listed).await;
    assert_field_refused("include", number("1"), listed).await;
    assert_field_refused("include", Json::Bool(true), listed).await;
    assert_field_refused("include", Json::Object(IndexMap::new()), listed).await;
    assert_field_refused(
        "timestamp_granularities",
        Json::List(vec![number("1")]),
        listed,
    )
    .await;
    assert_field_refused("temperature", list(&["hot"]), scalar).await;
    assert_field_refused("temperature", Json::Object(IndexMap::new()), scalar).await;
    assert_field_refused("bogus", text("x"), "is not one the OpenAI SDK sends").await;
    assert_scalar_fields().await;
    let file = prepare_data(
        None,
        "whisper-1",
        &IndexMap::from([(String::from("file"), text("x"))]),
    )
    .expect_err("the file is the audio file");
    assert!(matches!(&file, TranscriptionError::Field { field, .. } if field == "file"));
}

async fn assert_scalar_fields() {
    let flag = field_outcome("stream", Json::Bool(true))
        .await
        .expect("a boolean is a scalar");
    assert_eq!(part_values(&flag, "stream"), ["true"]);
    let none = field_outcome("language", Json::Null)
        .await
        .expect("null sends no part");
    assert!(part_values(&none, "language").is_empty());
}

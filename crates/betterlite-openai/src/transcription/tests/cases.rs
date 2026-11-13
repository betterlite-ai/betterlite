use std::str::FromStr as _;
use std::time::Duration;

use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_http::proxy::outbound::RequestBody;
use betterlite_models::decisions::Json;
use http::HeaderMap;
use indexmap::IndexMap;
use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::SecretString;
use serde_json::Number;
use uuid::Uuid;

use super::fixture::{AudioFixture, ConnectionOptions, Fixture, Recorder, Reply, connection};
use crate::api::audio_file_input::get_audio_file_name;
use crate::openai_base_config::Headers;
use crate::transcription::call::{
    TranscriptionCall, TranscriptionRequest, async_audio_transcriptions, audio_transcriptions,
    finish_transcription,
};
use crate::transcription::client::{ClientKind, TranscriptionConnection, select_client};
use crate::transcription::config::{
    TranscriptionFamily, complete_url, map_openai_params, supported_openai_params,
    transform_response, validate_environment,
};
use crate::transcription::duration::srt_vtt_duration;
use crate::transcription::error::TranscriptionError;
use crate::transcription::reply::{StoredDuration, TranscriptionReply};
use crate::transcription::request::{TranscriptionData, prepare_data};
use crate::transcription::transport::TranscriptionTransport as _;

#[derive(Debug, Clone, Copy)]
enum Case {
    WhisperMultipart,
    GptTranscriptionFields,
    TimestampsArray,
    VerboseJsonSegments,
    TextSrtVttBody,
    UpstreamError,
    WhisperDefaultVerboseJson,
    GptInclude,
    TextDuration,
    SrtDuration,
    VttDuration,
    WhisperEndpointAuth,
    RawTranscriptionHeaders,
    NonJsonContentType,
    ModelProjection,
}

#[rstest]
#[case::whisper_multipart(Case::WhisperMultipart)]
#[case::gpt_transcription_fields(Case::GptTranscriptionFields)]
#[case::timestamps_array(Case::TimestampsArray)]
#[case::verbose_json_segments(Case::VerboseJsonSegments)]
#[case::text_srt_vtt_body(Case::TextSrtVttBody)]
#[case::upstream_error(Case::UpstreamError)]
#[case::whisper_default_verbose_json(Case::WhisperDefaultVerboseJson)]
#[case::gpt_include(Case::GptInclude)]
#[case::text_duration(Case::TextDuration)]
#[case::srt_duration(Case::SrtDuration)]
#[case::vtt_duration(Case::VttDuration)]
#[case::whisper_endpoint_auth(Case::WhisperEndpointAuth)]
#[case::raw_transcription_headers(Case::RawTranscriptionHeaders)]
#[case::non_json_content_type(Case::NonJsonContentType)]
#[case::model_projection(Case::ModelProjection)]
#[tokio::test]
async fn transcription_cases(#[case] case: Case) {
    match case {
        Case::WhisperMultipart => whisper_multipart().await,
        Case::GptTranscriptionFields => gpt_transcription_fields(),
        Case::TimestampsArray => timestamps_array().await,
        Case::VerboseJsonSegments => verbose_json_segments().await,
        Case::TextSrtVttBody => text_srt_vtt_body().await,
        Case::UpstreamError => upstream_error().await,
        Case::WhisperDefaultVerboseJson => whisper_default_verbose_json(),
        Case::GptInclude => gpt_include().await,
        Case::TextDuration => text_duration().await,
        Case::SrtDuration => srt_duration(),
        Case::VttDuration => vtt_duration(),
        Case::WhisperEndpointAuth => whisper_endpoint_auth(),
        Case::RawTranscriptionHeaders => raw_transcription_headers().await,
        Case::NonJsonContentType => non_json_content_type(),
        Case::ModelProjection => model_projection().await,
    }
}

pub(super) const BOUNDARY: Uuid = Uuid::from_u128(7);

pub(super) struct Run {
    pub(super) outcome: Result<TranscriptionReply, TranscriptionError>,
    pub(super) recorder: Recorder,
}

pub(super) struct SyncCall {
    pub(super) model: &'static str,
    pub(super) family: Option<TranscriptionFamily>,
    pub(super) options: IndexMap<String, Json>,
    pub(super) data: Option<TranscriptionData>,
    pub(super) connection: TranscriptionConnection,
}

impl SyncCall {
    pub(super) fn whisper(
        options: IndexMap<String, Json>,
        connection: TranscriptionConnection,
    ) -> Self {
        Self {
            model: "whisper-1",
            family: Some(TranscriptionFamily::Whisper),
            options,
            data: None,
            connection,
        }
    }

    pub(super) fn explicit(data: TranscriptionData, connection: TranscriptionConnection) -> Self {
        Self {
            model: "",
            family: None,
            options: IndexMap::new(),
            data: Some(data),
            connection,
        }
    }
}

pub(super) struct Wire {
    pub(super) url: String,
    pub(super) headers: HeaderMap,
    pub(super) parts: Vec<MultipartPart>,
}

pub(super) fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

pub(super) fn number(value: &str) -> Json {
    Json::Number(Number::from_str(value).expect("the fixture number is JSON"))
}

pub(super) fn list(items: &[&str]) -> Json {
    Json::List(items.iter().map(|item| text(item)).collect())
}

pub(super) fn value(expected: Json) -> serde_json::Value {
    serde_json::to_value(expected).expect("the expected value is JSON")
}

pub(super) fn wire(fixture: &Fixture) -> Wire {
    let request = fixture.request();
    assert_eq!(request.method, http::Method::POST);
    assert_eq!(request.deadline, Duration::from_secs(30));
    let RequestBody::Multipart { parts, .. } = request.body else {
        panic!("the transcription call sends a multipart body");
    };
    Wire {
        url: request.url.to_string(),
        headers: request.headers,
        parts,
    }
}

fn header(wire: &Wire, name: &str) -> Option<String> {
    wire.headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

pub(super) fn part_values(parts: &[MultipartPart], name: &str) -> Vec<String> {
    parts
        .iter()
        .filter(|part| part.name == name)
        .map(|part| String::from_utf8(part.data.clone()).expect("a text part"))
        .collect()
}

fn file_part(parts: &[MultipartPart]) -> &MultipartPart {
    parts
        .iter()
        .find(|part| part.name == "file")
        .expect("the body carries the file part")
}

fn part_names(wire: &Wire) -> Vec<&str> {
    wire.parts.iter().map(|part| part.name.as_str()).collect()
}

pub(super) async fn run(fixture: &Fixture, audio: &AudioFixture, call: SyncCall) -> Run {
    let processed = audio.processed();
    let mut recorder = Recorder::default();
    let prepared = TranscriptionCall {
        input: get_audio_file_name(&audio.audio_file()),
        file: &processed,
        connection: call.connection,
        boundary: BOUNDARY,
    };
    let started = if let Some(data) = call.data {
        async_audio_transcriptions(&prepared, &data, &mut recorder)
    } else {
        let request = TranscriptionRequest {
            model: call.model,
            family: call.family,
            optional_params: call.options,
            atranscription: false,
        };
        audio_transcriptions(&prepared, &request, &mut recorder)
    };
    let outcome = match started {
        Ok((outbound, pending)) => {
            let sent = fixture.send(outbound).await;
            finish_transcription(pending, sent, &mut recorder)
        }
        Err(error) => Err(error),
    };
    Run { outcome, recorder }
}

pub(super) fn whisper_data(options: &IndexMap<String, Json>) -> TranscriptionData {
    prepare_data(Some(TranscriptionFamily::Whisper), "whisper-1", options)
        .expect("the whisper data builds")
}

pub(super) fn default_connection() -> TranscriptionConnection {
    connection(&ConnectionOptions {
        api_key: Some("sk-transcription"),
        environment_api_key: None,
        shared_session: false,
    })
}

fn assert_whisper_wire(wire: &Wire) {
    assert_eq!(
        wire.url,
        "https://transcription.test/v1/audio/transcriptions"
    );
    assert_eq!(
        header(wire, "authorization").as_deref(),
        Some("Bearer sk-transcription")
    );
    assert_eq!(header(wire, "accept").as_deref(), Some("application/json"));
    assert_eq!(
        part_names(wire),
        [
            "model",
            "language",
            "prompt",
            "response_format",
            "temperature",
            "file"
        ]
    );
    assert_eq!(part_values(&wire.parts, "model"), ["whisper-1"]);
    assert_eq!(part_values(&wire.parts, "language"), ["en"]);
    assert_eq!(part_values(&wire.parts, "prompt"), ["greeting"]);
    // whisper fills verbose_json in because the caller left response_format unset
    assert_eq!(
        part_values(&wire.parts, "response_format"),
        ["verbose_json"]
    );
    assert_eq!(part_values(&wire.parts, "temperature"), ["0.3"]);
    let file = file_part(&wire.parts);
    assert_eq!(file.file_name.as_deref(), Some("a.wav"));
    assert_eq!(file.content_type.as_deref(), Some("audio/wav"));
    assert_eq!(file.data, b"abc");
}

fn assert_whisper_reply(reply: &TranscriptionReply) {
    assert_eq!(reply.response.text.as_deref(), Some("hi"));
    assert_eq!(
        reply.response.extra.get("language"),
        Some(&value(text("en")))
    );
    assert_eq!(
        reply.response.extra.get("duration"),
        Some(&value(number("1.5")))
    );
    assert_eq!(reply.model.as_deref(), Some("whisper-1"));
    assert_eq!(reply.custom_llm_provider, "openai");
}

fn assert_whisper_log(recorder: &Recorder) {
    assert_eq!(recorder.pre_calls.len(), 1);
    let pre = &recorder.pre_calls[0];
    assert_eq!(pre.api_base, "https://transcription.test/v1/");
    assert_eq!(pre.data.get("model"), Some(&text("whisper-1")));
    assert_eq!(pre.data.get("response_format"), Some(&text("verbose_json")));
    assert_eq!(recorder.post_calls.len(), 1);
    let post = &recorder.post_calls[0];
    // B1001's get_audio_file_name spells a tuple as its content kinds
    assert_eq!(post.input, "(a.wav, bytes)");
    assert_eq!(post.data.as_ref(), Some(&pre.data));
}

async fn whisper_multipart() {
    let audio = AudioFixture::new("a.wav", b"abc", "audio/wav");
    let fixture = Fixture::new(Reply::json(
        r#"{"text":"hi","language":"en","duration":1.5}"#,
    ));
    let options = IndexMap::from([
        (String::from("language"), text("en")),
        (String::from("prompt"), text("greeting")),
        (String::from("temperature"), number("0.3")),
    ]);
    let run = run(
        &fixture,
        &audio,
        SyncCall::whisper(options, default_connection()),
    )
    .await;
    assert_eq!(fixture.sends(), 1);
    let sent = wire(&fixture);
    assert_whisper_wire(&sent);
    let reply = run.outcome.expect("the 200 reply is a success");
    assert_whisper_reply(&reply);
    assert_whisper_log(&run.recorder);
}

fn gpt_transcription_fields() {
    assert_supported_params();
    assert_mapped_params();
}

fn assert_supported_params() {
    assert_eq!(
        supported_openai_params(TranscriptionFamily::Whisper),
        [
            "language",
            "prompt",
            "response_format",
            "temperature",
            "timestamp_granularities"
        ]
    );
    assert_eq!(
        supported_openai_params(TranscriptionFamily::Gpt),
        [
            "language",
            "prompt",
            "response_format",
            "temperature",
            "include"
        ]
    );
}

fn requested_params() -> IndexMap<String, Json> {
    IndexMap::from([
        (String::from("language"), text("en")),
        (String::from("prompt"), text("greeting")),
        (String::from("response_format"), text("json")),
        (String::from("temperature"), number("0.2")),
        (String::from("timestamp_granularities"), list(&["word"])),
        (String::from("include"), list(&["logprobs"])),
        (String::from("unsupported"), Json::Bool(true)),
    ])
}

fn assert_mapped_params() {
    let requested = requested_params();
    let whisper = map_openai_params(TranscriptionFamily::Whisper, &requested);
    let whisper_names: Vec<&str> = whisper.keys().map(String::as_str).collect();
    assert_eq!(
        whisper_names,
        [
            "language",
            "prompt",
            "response_format",
            "temperature",
            "timestamp_granularities"
        ]
    );
    let gpt = map_openai_params(TranscriptionFamily::Gpt, &requested);
    let gpt_names: Vec<&str> = gpt.keys().map(String::as_str).collect();
    assert_eq!(
        gpt_names,
        [
            "language",
            "prompt",
            "response_format",
            "temperature",
            "include"
        ]
    );
    // the mapping copies into a new dict and never touches the caller's
    assert_eq!(requested.len(), 7);
}

async fn timestamps_array() {
    let audio = AudioFixture::new("a.wav", b"abc", "audio/wav");
    let fixture = Fixture::new(Reply::json(r#"{"text":"hi"}"#));
    let options = IndexMap::from([(
        String::from("timestamp_granularities"),
        list(&["word", "segment"]),
    )]);
    let run = run(
        &fixture,
        &audio,
        SyncCall::whisper(options, default_connection()),
    )
    .await;
    run.outcome.expect("the 200 reply is a success");
    let sent = wire(&fixture);
    assert_eq!(
        part_values(&sent.parts, "timestamp_granularities[]"),
        ["word", "segment"]
    );
    assert_eq!(
        part_names(&sent),
        [
            "model",
            "response_format",
            "timestamp_granularities[]",
            "timestamp_granularities[]",
            "file"
        ]
    );
}

async fn verbose_json_segments() {
    let audio = AudioFixture::new("a.wav", b"abc", "audio/wav");
    let body = r#"{"task":"transcribe","language":"en","duration":1.5,"text":"hi","words":[{"word":"hi","start":0.0,"end":1.0}],"segments":[{"id":0,"start":0.0,"end":1.5,"text":"hi"}],"usage":{"type":"duration","seconds":1.5}}"#;
    let fixture = Fixture::new(Reply::json(body));
    let run = run(
        &fixture,
        &audio,
        SyncCall::whisper(IndexMap::new(), default_connection()),
    )
    .await;
    let reply = run.outcome.expect("the 200 reply is a success");
    assert_eq!(reply.response.text.as_deref(), Some("hi"));
    for name in ["task", "language", "duration", "words", "segments", "usage"] {
        assert!(
            reply.response.extra.contains_key(name),
            "the response keeps {name}"
        );
    }
    assert_eq!(
        reply.response.extra.get("task"),
        Some(&value(text("transcribe")))
    );
    assert_eq!(
        reply.response.extra.get("usage"),
        Some(&value(Json::Object(IndexMap::from([
            (String::from("type"), text("duration")),
            (String::from("seconds"), number("1.5")),
        ]))))
    );
    // the SDK's model_dump lists every declared segment field, null when the body has none
    let dumped = r#"{"duration":1.5,"language":"en","text":"hi","segments":[{"id":0,"avg_logprob":null,"compression_ratio":null,"end":1.5,"no_speech_prob":null,"seek":null,"start":0.0,"temperature":null,"text":"hi","tokens":null}],"usage":{"seconds":1.5,"type":"duration"},"words":[{"end":1.0,"start":0.0,"word":"hi"}],"task":"transcribe"}"#;
    let original = serde_json::from_str::<Json>(dumped).expect("the dump is JSON");
    assert_eq!(reply.original_response, original);
    // LiteLLM builds the response from the same dump, so its segments carry the nulls too
    let segments = value(original.clone())["segments"].clone();
    assert_eq!(reply.response.extra.get("segments"), Some(&segments));
    assert_eq!(run.recorder.post_calls[0].original_response, original);
}

fn srt_options() -> IndexMap<String, Json> {
    IndexMap::from([(String::from("response_format"), text("srt"))])
}

async fn sync_text_reply(audio: &AudioFixture) -> TranscriptionReply {
    let srt = "1\n00:00:01,000 --> 00:00:02,000\nhi\n";
    let fixture = Fixture::new(Reply::text(srt));
    let run = run(
        &fixture,
        audio,
        SyncCall::whisper(srt_options(), default_connection()),
    )
    .await;
    run.outcome.expect("the 200 reply is text")
}

async fn text_srt_vtt_body() {
    let audio = AudioFixture::new("a.wav", b"abc", "audio/wav");
    let srt = "1\n00:00:01,000 --> 00:00:02,000\nhi\n";
    let sync_reply = sync_text_reply(&audio).await;
    assert_eq!(sync_reply.response.text.as_deref(), Some(srt));
    // the sync textual reply carries no duration metadata and no async duration key
    assert_eq!(sync_reply.duration, StoredDuration::Absent);
    assert_eq!(
        sync_reply.original_response,
        Json::Object(IndexMap::from([
            (String::from("text"), text(srt)),
            (String::from("usage"), Json::Null),
        ]))
    );
    let vtt = "WEBVTT\n\n00:00:01.500 --> 00:00:03.250\nhi";
    let fixture = Fixture::new(Reply::text(vtt));
    let options = IndexMap::from([(String::from("response_format"), text("vtt"))]);
    let run = run(
        &fixture,
        &audio,
        SyncCall::explicit(whisper_data(&options), shared_connection()),
    )
    .await;
    let async_reply = run.outcome.expect("the 200 reply is text");
    assert_eq!(async_reply.response.text.as_deref(), Some(vtt));
    assert_eq!(
        async_reply.duration,
        StoredDuration::Known(Some(decimal("3.25")))
    );
}

fn error_reply() -> Reply {
    let body = r#"{"error":{"message":"The model does not exist","type":"invalid_request_error","param":"model","code":"model_not_found"}}"#;
    Reply {
        status: 404,
        headers: vec![(
            String::from("content-type"),
            String::from("application/json"),
        )],
        body: body.as_bytes().to_vec(),
    }
}

async fn upstream_error() {
    let audio = AudioFixture::new("a.wav", b"abc", "audio/wav");
    let sync_fixture = Fixture::new(error_reply());
    let sync_run = run(
        &sync_fixture,
        &audio,
        SyncCall::whisper(IndexMap::new(), default_connection()),
    )
    .await;
    let Err(TranscriptionError::Status(status)) = sync_run.outcome else {
        panic!("the 404 is the SDK's status error");
    };
    assert_eq!(status.status, 404);
    assert_eq!(status.error_type.as_deref(), Some("invalid_request_error"));
    assert_eq!(status.param.as_deref(), Some("model"));
    assert_eq!(status.code.as_deref(), Some("model_not_found"));
    assert!(sync_run.recorder.post_calls.is_empty());
    assert_async_error(&audio).await;
}

async fn assert_async_error(audio: &AudioFixture) {
    let fixture = Fixture::new(error_reply());
    let run = run(
        &fixture,
        audio,
        SyncCall::explicit(whisper_data(&IndexMap::new()), default_connection()),
    )
    .await;
    let Err(TranscriptionError::Status(status)) = run.outcome else {
        panic!("the 404 is an error");
    };
    // the async path logs and rethrows the same error unwrapped
    assert_eq!(status.status, 404);
    assert_eq!(run.recorder.post_calls.len(), 1);
    let post = &run.recorder.post_calls[0];
    assert_eq!(post.input, "(a.wav, bytes)");
    assert!(post.data.is_none());
    assert!(matches!(&post.original_response, Json::Text(text) if text.contains("404")));
}

fn whisper_default_verbose_json() {
    let absent = prepare_data(
        Some(TranscriptionFamily::Whisper),
        "whisper-1",
        &IndexMap::new(),
    )
    .expect("the whisper data builds");
    assert_eq!(
        absent.options.get("response_format"),
        Some(&text("verbose_json"))
    );
    assert_explicit_formats();
    assert_other_families();
}

fn assert_explicit_formats() {
    let explicit = IndexMap::from([(String::from("response_format"), text("json"))]);
    let data = prepare_data(Some(TranscriptionFamily::Whisper), "whisper-1", &explicit)
        .expect("the whisper data builds");
    assert_eq!(data.options.get("response_format"), Some(&text("json")));
    // an explicit null survives: the default is only for a missing key
    let null = IndexMap::from([(String::from("response_format"), Json::Null)]);
    let found = prepare_data(Some(TranscriptionFamily::Whisper), "whisper-1", &null)
        .expect("the whisper data builds");
    assert_eq!(found.options.get("response_format"), Some(&Json::Null));
}

fn assert_other_families() {
    let gpt = prepare_data(
        Some(TranscriptionFamily::Gpt),
        "gpt-4o-transcribe",
        &IndexMap::new(),
    )
    .expect("the gpt data builds");
    assert!(!gpt.options.contains_key("response_format"));
    let raw = prepare_data(None, "whisper-1", &IndexMap::new()).expect("the raw data builds");
    assert!(!raw.options.contains_key("response_format"));
}

fn gpt_call(options: IndexMap<String, Json>) -> SyncCall {
    SyncCall {
        model: "gpt-4o-transcribe",
        family: Some(TranscriptionFamily::Gpt),
        options,
        data: None,
        connection: default_connection(),
    }
}

async fn gpt_include() {
    let audio = AudioFixture::new("voice.wav", b"abc", "audio/wav");
    let fixture = Fixture::new(Reply::json(r#"{"text":"hi"}"#));
    let options = IndexMap::from([
        (String::from("include"), list(&["logprobs"])),
        (String::from("response_format"), text("json")),
    ]);
    let run = run(&fixture, &audio, gpt_call(options)).await;
    run.outcome.expect("the 200 reply is a success");
    let sent = wire(&fixture);
    assert_eq!(part_values(&sent.parts, "include[]"), ["logprobs"]);
    assert_eq!(part_values(&sent.parts, "response_format"), ["json"]);
    assert!(part_values(&sent.parts, "timestamp_granularities[]").is_empty());
}

fn decimal(source: &str) -> Decimal {
    Decimal::from_str(source).expect("the duration is a decimal")
}

fn srt_body() -> &'static str {
    "1\n00:00:02,500 --> 00:00:04,000\nhi\n"
}

async fn text_duration() {
    let audio = AudioFixture::new("a.wav", b"abc", "audio/wav");
    assert_async_text_duration(&audio, srt_body(), srt_options(), Some("4.0")).await;
    let options = IndexMap::from([(String::from("response_format"), text("text"))]);
    assert_async_text_duration(&audio, "just words", options, None).await;
    // the reply decides, not the request: a verbose_json request answered with plain text is a textual reply
    assert_async_text_duration(&audio, srt_body(), IndexMap::new(), Some("4.0")).await;
    assert_sync_reply_has_no_duration(&audio).await;
    assert_async_json_reply_has_no_duration(&audio).await;
}

async fn assert_async_json_reply_has_no_duration(audio: &AudioFixture) {
    let fixture = Fixture::new(Reply::json(r#"{"text":"hi"}"#));
    let run = run(
        &fixture,
        audio,
        SyncCall::explicit(whisper_data(&IndexMap::new()), default_connection()),
    )
    .await;
    let reply = run.outcome.expect("the 200 reply is json");
    assert_eq!(reply.duration, StoredDuration::Absent);
    let Json::Object(fields) = &reply.original_response else {
        panic!("the reply is an object");
    };
    assert!(!fields.contains_key("_audio_transcription_duration"));
}

async fn assert_async_text_duration(
    audio: &AudioFixture,
    body: &str,
    options: IndexMap<String, Json>,
    duration: Option<&str>,
) {
    let fixture = Fixture::new(Reply::text(body));
    let run = run(
        &fixture,
        audio,
        SyncCall::explicit(whisper_data(&options), default_connection()),
    )
    .await;
    let reply = run.outcome.expect("the 200 reply is text");
    let recorded = duration.map_or(Json::Null, number);
    assert_eq!(reply.duration, StoredDuration::Known(duration.map(decimal)));
    assert_eq!(
        reply.original_response,
        Json::Object(IndexMap::from([
            (String::from("text"), text(body)),
            (String::from("usage"), Json::Null),
            (String::from("_audio_transcription_duration"), recorded),
        ]))
    );
}

async fn assert_sync_reply_has_no_duration(audio: &AudioFixture) {
    let fixture = Fixture::new(Reply::text(srt_body()));
    let run = run(
        &fixture,
        audio,
        SyncCall::whisper(srt_options(), default_connection()),
    )
    .await;
    assert_eq!(
        run.outcome.expect("the 200 reply is text").duration,
        StoredDuration::Absent
    );
}

fn srt_duration() {
    assert_eq!(
        srt_vtt_duration("00:00:01,000 --> 00:00:02,000\nhi"),
        Some(Decimal::from(2_u32))
    );
    // the largest stamp wins even when it comes before a smaller one
    assert_eq!(
        srt_vtt_duration("00:10:00,000\n00:00:02,000"),
        Some(Decimal::from(600_u32))
    );
    assert_eq!(srt_vtt_duration("00:00:01,001"), Some(decimal("1.001")));
    assert_eq!(srt_vtt_duration("no timestamps here"), None);
    // minutes and seconds are not range checked, and the fraction is exact
    assert_eq!(
        srt_vtt_duration("99:99:99,999"),
        Some(decimal("362439.999"))
    );
}

fn vtt_duration() {
    assert_eq!(
        srt_vtt_duration("00:00:01.500 --> 00:00:02.000"),
        Some(Decimal::from(2_u32))
    );
    assert_eq!(srt_vtt_duration("00:00:01,500"), Some(decimal("1.5")));
    // a stamp with only two fraction digits does not match, so the three-digit one wins
    assert_eq!(
        srt_vtt_duration("00:00:09.50 and 00:00:01,250"),
        Some(decimal("1.25"))
    );
    assert_eq!(srt_vtt_duration(""), None);
}

fn key(secret: &str) -> SecretString {
    SecretString::from(secret.to_owned())
}

fn whisper_endpoint_auth() {
    assert_complete_urls();
    assert_auth_headers();
}

fn assert_complete_urls() {
    assert_eq!(complete_url(None), "/v1/audio/transcriptions");
    assert_eq!(complete_url(Some("")), "/v1/audio/transcriptions");
    assert_eq!(
        complete_url(Some("https://speech.test/v1")),
        "https://speech.test/v1/audio/transcriptions"
    );
    assert_eq!(
        complete_url(Some("https://speech.test/v1/")),
        "https://speech.test/v1/audio/transcriptions"
    );
    assert_eq!(
        complete_url(Some("https://speech.test/v2")),
        "https://speech.test/v2/v1/audio/transcriptions"
    );
    assert_eq!(
        complete_url(Some("https://speech.test")),
        "https://speech.test/v1/audio/transcriptions"
    );
}

fn assert_auth_headers() {
    let caller = Headers::from([(String::from("Authorization"), String::from("Bearer caller"))]);
    let explicit = key("sk-explicit");
    let environment = key("sk-environment");
    let headers = validate_environment(&caller, Some(&explicit), Some(&environment));
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer sk-explicit")
    );
    let inherited = validate_environment(&Headers::new(), None, Some(&environment));
    assert_eq!(
        inherited.get("Authorization").map(String::as_str),
        Some("Bearer sk-environment")
    );
    let missing = validate_environment(&Headers::new(), None, None);
    assert_eq!(
        missing.get("Authorization").map(String::as_str),
        Some("Bearer None")
    );
}

fn assert_provider_headers(reply: &TranscriptionReply) {
    assert_eq!(
        reply
            .provider_headers
            .headers
            .get("x-request-id")
            .map(String::as_str),
        Some("req-1")
    );
    assert_eq!(
        reply
            .provider_headers
            .additional_headers
            .get("llm_provider-x-request-id"),
        Some(&text("req-1"))
    );
    assert_eq!(
        reply
            .provider_headers
            .additional_headers
            .get("llm_provider-content-type"),
        Some(&text("application/json"))
    );
}

async fn raw_transcription_headers() {
    let audio = AudioFixture::new("a.wav", b"abc", "audio/wav");
    assert_headers(&audio, true).await;
    assert_headers(&audio, false).await;
}

async fn assert_headers(audio: &AudioFixture, sync: bool) {
    let fixture = Fixture::new(Reply::json(r#"{"text":"hi"}"#));
    let run = if sync {
        run(
            &fixture,
            audio,
            SyncCall::whisper(IndexMap::new(), default_connection()),
        )
        .await
    } else {
        run(
            &fixture,
            audio,
            SyncCall::explicit(whisper_data(&IndexMap::new()), default_connection()),
        )
        .await
    };
    let reply = run.outcome.expect("the 200 reply is a success");
    assert_provider_headers(&reply);
    assert_eq!(run.recorder.response_headers.len(), 1);
    assert_eq!(
        run.recorder.response_headers[0]
            .get("x-request-id")
            .map(String::as_str),
        Some("req-1")
    );
    assert_eq!(run.recorder.post_calls.len(), 1);
}

fn non_json_content_type() {
    let json = transform_response(b"{\"text\":\"hi\"}", Some("application/json"))
        .expect("the JSON body is a transcription");
    assert_eq!(json.text.as_deref(), Some("hi"));
    let plain = transform_response(b"not json at all", Some("text/plain"))
        .expect("a plain body becomes its text");
    assert_eq!(plain.text.as_deref(), Some("not json at all"));
    let html = transform_response(b"not json at all", Some("TEXT/HTML"))
        .expect("any other content type becomes text");
    assert_eq!(html.text.as_deref(), Some("not json at all"));
    let parsed = transform_response(b"{\"text\":\"hi\"}", Some("text/plain"))
        .expect("the parsed object is a transcription");
    assert_eq!(parsed.text.as_deref(), Some("hi"));
    assert_transform_errors();
}

fn assert_transform_errors() {
    let json_error =
        transform_response(b"not json at all", Some("Application/JSON; charset=utf-8"))
            .expect_err("a JSON content type rethrows the decode error");
    assert!(matches!(json_error, TranscriptionError::Json { .. }));
    // a JSON object without text or usage is not a response
    let empty = transform_response(b"{\"foo\":\"bar\"}", Some("text/plain"))
        .expect_err("an unexpected object is not textual");
    assert!(matches!(empty, TranscriptionError::NoResponseFields));
    // TranscriptionResponse(**json) takes the text keyword only, so any other key raises
    let extra = transform_response(b"{\"text\":\"hi\",\"language\":\"en\"}", Some("text/plain"))
        .expect_err("a field beyond text is not a TranscriptionResponse argument");
    assert!(matches!(&extra, TranscriptionError::Field { field, .. } if field == "language"));
}

async fn model_projection() {
    let audio = AudioFixture::new("a.wav", b"abc", "audio/wav");
    assert_model(&audio, None, Some("whisper-1")).await;
    let mut gpt = whisper_data(&IndexMap::new());
    gpt.model = Some(text("gpt-4o-transcribe"));
    assert_model(&audio, Some(gpt), Some("gpt-4o-transcribe")).await;
    let missing = TranscriptionData {
        model: None,
        options: IndexMap::new(),
    };
    assert_model(&audio, Some(missing), Some("whisper-1")).await;
    let null = TranscriptionData {
        model: Some(Json::Null),
        options: IndexMap::new(),
    };
    assert_model(&audio, Some(null), None).await;
    assert_shared_sessions();
}

async fn assert_model(
    audio: &AudioFixture,
    data: Option<TranscriptionData>,
    expected: Option<&str>,
) {
    let fixture = Fixture::new(Reply::json(r#"{"text":"hi"}"#));
    let run = match data {
        Some(provided) => {
            run(
                &fixture,
                audio,
                SyncCall::explicit(provided, default_connection()),
            )
            .await
        }
        None => {
            run(
                &fixture,
                audio,
                SyncCall::whisper(IndexMap::new(), default_connection()),
            )
            .await
        }
    };
    assert_eq!(
        run.outcome
            .expect("the 200 reply is a success")
            .model
            .as_deref(),
        expected
    );
}

fn assert_shared_sessions() {
    let sync = select_client(&shared_connection(), ClientKind::Sync).expect("the client builds");
    assert!(sync.shared_session.is_none());
    assert_eq!(sync.kind, ClientKind::Sync);
    let async_client =
        select_client(&shared_connection(), ClientKind::Async).expect("the client builds");
    assert!(async_client.shared_session.is_some());
    assert_eq!(async_client.kind, ClientKind::Async);
}

fn shared_connection() -> TranscriptionConnection {
    connection(&ConnectionOptions {
        api_key: Some("sk-transcription"),
        environment_api_key: None,
        shared_session: true,
    })
}

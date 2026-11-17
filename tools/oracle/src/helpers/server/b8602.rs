use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::stream_chunks::fallback::{
    FallbackMeta, client_requested_model, fallback_event_frame, fallback_metadata,
    include_fallback_errors, is_positive_int_like,
};
use betterlite_server::stream_chunks::frames::{
    ends_with_frame_delimiter, pop_sse_frame, sse_data_frame,
};
use betterlite_server::stream_chunks::restamp::{RestampInput, RouterCheck, restamp_chunk_model};
use betterlite_server::stream_chunks::serialize::{
    chunk_response_text, is_usage_artifact, serialize_chunk,
};
use betterlite_server::stream_chunks::{ChunkKind, StreamChunk, StreamItem};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    ClientModel {
        requests: Vec<JsonObject>,
    },
    IntLike {
        values: Vec<Json>,
    },
    IncludeErrors {
        expose: bool,
        requests: Vec<JsonObject>,
    },
    FallbackMeta {
        hidden: Vec<Json>,
    },
    FallbackEvent {
        events: Vec<EventCase>,
    },
    Restamp {
        restamps: Vec<RestampCase>,
    },
    ChunkView {
        view: ViewName,
        chunks: Vec<ChunkCase>,
    },
    Frames {
        payloads: Vec<String>,
        buffers: Vec<String>,
        endings: Vec<String>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EventCase {
    model: Option<String>,
    errors: Vec<JsonObject>,
    created: i64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RestampCase {
    kind: ChunkKindCase,
    body: JsonObject,
    requested_model: String,
    request_data: JsonObject,
    fallback_attempted: bool,
    fallback_model: Option<String>,
    mismatch_logged: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChunkCase {
    kind: ChunkKindCase,
    body: JsonObject,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ChunkKindCase {
    Stream,
    Full,
    Event,
    Plain,
}

impl ChunkKindCase {
    const fn chunk_kind(self) -> ChunkKind {
        match self {
            Self::Stream => ChunkKind::ModelStream,
            Self::Full => ChunkKind::ModelFull,
            Self::Event => ChunkKind::Event,
            Self::Plain => ChunkKind::Plain,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ViewName {
    Serialized,
    UsageArtifact,
    Text,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let answer = match input {
            Input::ClientModel { requests } => client_models(requests),
            Input::IntLike { values } => int_likes(values),
            Input::IncludeErrors { expose, requests } => include_error_flags(*expose, requests),
            Input::FallbackMeta { hidden } => fallback_metas(hidden),
            Input::FallbackEvent { events } => fallback_events(events),
            Input::Restamp { restamps } => restamp_answers(restamps),
            Input::ChunkView { view, chunks } => chunk_views(*view, chunks),
            Input::Frames {
                payloads,
                buffers,
                endings,
            } => frames(payloads, buffers, endings),
        };
        Ok(serde_json::to_value(answer).unwrap_or_default())
    }
}

fn client_models(requests: &[JsonObject]) -> Json {
    let answers = requests
        .iter()
        .map(|request| Json::Text(client_requested_model(request)))
        .collect();
    Json::List(answers)
}

fn int_likes(values: &[Json]) -> Json {
    let answers = values
        .iter()
        .map(|value| Json::Bool(is_positive_int_like(Some(value))))
        .collect();
    Json::List(answers)
}

fn include_error_flags(expose: bool, requests: &[JsonObject]) -> Json {
    let answers = requests
        .iter()
        .map(|request| Json::Bool(include_fallback_errors(expose, request)))
        .collect();
    Json::List(answers)
}

fn fallback_metas(hidden: &[Json]) -> Json {
    let answers = hidden.iter().map(fallback_meta_answer).collect();
    Json::List(answers)
}

fn fallback_meta_answer(hidden: &Json) -> Json {
    let meta = match hidden {
        Json::Object(fields) => fallback_metadata(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            FallbackMeta::default()
        }
    };
    let errors = Json::List(meta.errors.into_iter().map(Json::Object).collect());
    let mut answer = JsonObject::new();
    answer.insert("attempted".to_owned(), Json::Bool(meta.attempted));
    answer.insert(
        "model".to_owned(),
        meta.model.map_or(Json::Null, Json::Text),
    );
    answer.insert("errors".to_owned(), errors);
    Json::Object(answer)
}

fn fallback_events(events: &[EventCase]) -> Json {
    let answers = events.iter().map(fallback_event_answer).collect();
    Json::List(answers)
}

fn fallback_event_answer(case: &EventCase) -> Json {
    let frame = fallback_event_frame(case.model.as_deref(), &case.errors, case.created);
    let mut answer = JsonObject::new();
    answer.insert("payload".to_owned(), frame_payload(&frame));
    answer.insert("prefix".to_owned(), Json::Bool(frame.starts_with("data: ")));
    answer.insert("suffix".to_owned(), Json::Bool(frame.ends_with("\n\n")));
    Json::Object(answer)
}

fn frame_payload(frame: &str) -> Json {
    let body = frame
        .strip_prefix("data: ")
        .and_then(|rest| rest.strip_suffix("\n\n"));
    let parsed = body.and_then(|body| serde_json::from_str::<Json>(body).ok());
    parsed.unwrap_or(Json::Null)
}

fn restamp_answers(cases: &[RestampCase]) -> Json {
    let answers = cases.iter().map(restamp_answer).collect();
    Json::List(answers)
}

fn restamp_answer(case: &RestampCase) -> Json {
    let mut item = StreamItem::Chunk(stream_chunk(case.kind, &case.body));
    let is_model_router: RouterCheck = |_model: &str| false;
    let input = RestampInput {
        requested_model: &case.requested_model,
        request_data: &case.request_data,
        fallback_attempted: case.fallback_attempted,
        fallback_model: case.fallback_model.as_deref(),
        is_model_router,
    };
    let logged = restamp_chunk_model(&mut item, &input, case.mismatch_logged);
    let model = match &item {
        StreamItem::Chunk(chunk) => chunk.body.get("model").cloned(),
        StreamItem::Bytes(_) | StreamItem::Text(_) => None,
    };
    let mut answer = JsonObject::new();
    answer.insert("model".to_owned(), model.unwrap_or(Json::Null));
    answer.insert("logged".to_owned(), Json::Bool(logged));
    Json::Object(answer)
}

fn chunk_views(view: ViewName, chunks: &[ChunkCase]) -> Json {
    let answers = chunks.iter().map(|case| chunk_view(case, view)).collect();
    Json::List(answers)
}

fn chunk_view(case: &ChunkCase, view: ViewName) -> Json {
    let chunk = stream_chunk(case.kind, &case.body);
    match view {
        ViewName::Serialized => {
            let text = serialize_chunk(&chunk);
            serde_json::from_str(&text).unwrap_or(Json::Null)
        }
        ViewName::UsageArtifact => Json::Bool(is_usage_artifact(&chunk)),
        ViewName::Text => Json::Text(chunk_response_text(&chunk)),
    }
}

fn stream_chunk(kind: ChunkKindCase, body: &JsonObject) -> StreamChunk {
    StreamChunk {
        body: body.clone(),
        kind: kind.chunk_kind(),
        hidden: JsonObject::new(),
    }
}

fn frames(payloads: &[String], buffers: &[String], endings: &[String]) -> Json {
    let data_frames = payloads
        .iter()
        .map(|payload| Json::Text(sse_data_frame(payload)))
        .collect();
    let splits = buffers
        .iter()
        .map(|buffer| split_answer(buffer.as_str()))
        .collect();
    let ends = endings
        .iter()
        .map(|text| Json::Bool(ends_with_frame_delimiter(text)))
        .collect();
    let mut answer = JsonObject::new();
    answer.insert("data_frames".to_owned(), Json::List(data_frames));
    answer.insert("splits".to_owned(), Json::List(splits));
    answer.insert("ends".to_owned(), Json::List(ends));
    Json::Object(answer)
}

fn split_answer(buffer: &str) -> Json {
    let (frame, rest) = pop_sse_frame(buffer);
    let mut answer = JsonObject::new();
    answer.insert("frame".to_owned(), frame.map_or(Json::Null, Json::Text));
    answer.insert("rest".to_owned(), Json::Text(rest));
    Json::Object(answer)
}

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::transcription_protocol::events::UuidEventIds;
use betterlite_core::api::transcription_protocol::usage::TranscriptionUsage;
use betterlite_core::api::vertex_chirp_protocol::events::{
    ChirpBackendEvent, ChirpEvent, ChirpSpeechEvent, ChirpStreamingResponse, ChirpStreamingResult,
    ChirpTurnDiscarded,
};
use betterlite_core::api::vertex_chirp_protocol::transformer::{
    ChirpEventTransformer, ItemIdSource,
};
use betterlite_core::api::vertex_chirp_protocol::words::new_words;
use betterlite_core::api::vertex_chirp_protocol::{ChirpSessionConfig, parse_chirp_session_update};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::helpers::{Helper, HelperResult, OrHelperError as _};

mod tests;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Function {
    Punctuation {
        pairs: Vec<PunctuationPair>,
    },
    Trace {
        session_id: String,
        session: SessionInput,
        frames: Vec<FrameInput>,
    },
    Sessions {
        expected_model: String,
        payloads: Vec<String>,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PunctuationPair {
    previous: String,
    current: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionInput {
    model: String,
    language: Option<String>,
    sample_rate: i64,
    server_vad: bool,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum FrameInput {
    Response {
        speech_event: ChirpSpeechEvent,
        results: Vec<ResultInput>,
        #[serde(with = "rust_decimal::serde::float")]
        billed_seconds: Decimal,
    },
    Configured,
    TurnFinished,
    TurnDiscarded {
        #[serde(with = "rust_decimal::serde::float")]
        billed_seconds: Decimal,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResultInput {
    transcript: String,
    is_final: bool,
}

#[derive(Serialize)]
struct PunctuationOutput {
    answers: Vec<String>,
}

#[derive(Serialize)]
struct TraceOutput {
    events: Vec<ChirpEvent>,
    usage: Option<TranscriptionUsage>,
    repeated_usage: Option<TranscriptionUsage>,
}

#[derive(Serialize)]
struct SessionsOutput {
    sessions: Vec<Option<ChirpSessionConfig>>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Function;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Function::Punctuation { pairs } => punctuation(pairs),
            Function::Trace {
                session_id,
                session,
                frames,
            } => trace(session_id, session, frames),
            Function::Sessions {
                expected_model,
                payloads,
            } => sessions(expected_model, payloads),
        }
    }
}

fn punctuation(pairs: &[PunctuationPair]) -> HelperResult {
    let output = PunctuationOutput {
        answers: pairs
            .iter()
            .map(|pair| new_words(&pair.previous, &pair.current))
            .collect(),
    };
    serde_json::to_value(&output).or_value_error()
}

fn trace(session_id: &str, session: &SessionInput, frames: &[FrameInput]) -> HelperResult {
    let mut transformer = ChirpEventTransformer::new(UuidEventIds, SequenceItemIds::default());
    transformer.configure(session.config(), session_id);
    let mut events = Vec::new();
    for frame in frames {
        events.extend(transformer.transform(frame.event()).or_value_error()?);
    }
    let usage = transformer.take_unbilled_usage().or_value_error()?;
    let repeated_usage = transformer.take_unbilled_usage().or_value_error()?;
    let output = TraceOutput {
        events,
        usage,
        repeated_usage,
    };
    serde_json::to_value(&output).or_value_error()
}

fn sessions(expected_model: &str, payloads: &[String]) -> HelperResult {
    let limits = ApiLimits::default();
    let configs = payloads
        .iter()
        .map(|payload| parse_chirp_session_update(payload.as_bytes(), expected_model, limits).ok())
        .collect();
    let output = SessionsOutput { sessions: configs };
    serde_json::to_value(&output).or_value_error()
}

impl FrameInput {
    fn event(&self) -> ChirpBackendEvent {
        match self {
            Self::Response {
                speech_event,
                results,
                billed_seconds,
            } => ChirpBackendEvent::Response(ChirpStreamingResponse {
                speech_event: *speech_event,
                results: results
                    .iter()
                    .map(|result| ChirpStreamingResult {
                        transcript: result.transcript.clone(),
                        is_final: result.is_final,
                    })
                    .collect(),
                billed_seconds: *billed_seconds,
            }),
            Self::Configured => ChirpBackendEvent::Configured,
            Self::TurnFinished => ChirpBackendEvent::TurnFinished,
            Self::TurnDiscarded { billed_seconds } => {
                ChirpBackendEvent::TurnDiscarded(ChirpTurnDiscarded {
                    billed_seconds: *billed_seconds,
                })
            }
        }
    }
}

impl SessionInput {
    fn config(&self) -> ChirpSessionConfig {
        ChirpSessionConfig {
            model: self.model.clone(),
            language: self.language.clone(),
            sample_rate: self.sample_rate,
            server_vad: self.server_vad,
        }
    }
}

#[derive(Default)]
struct SequenceItemIds {
    next: usize,
}

impl ItemIdSource for SequenceItemIds {
    fn next_item_id(&mut self) -> String {
        let identifier = format!("item_{}", self.next);
        self.next = self.next.saturating_add(1);
        identifier
    }
}

use std::fmt::Display;

use betterlite_bedrock::realtime_output::NovaRealtimeOutput;
use betterlite_bedrock::realtime_output::events::{
    AudioOutput, ContentStart, DeltaType, OutputEvent, OutputIds, ResponseDelta, TextOutput,
};
use betterlite_bedrock::realtime_session::RealtimeSession;
use betterlite_bedrock::realtime_session::records::{UsageEvent, UsageTokens};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::services_realtime_session::FixedIds;
use crate::helpers::{Helper, HelperError, HelperResult};

mod tests;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    ids: Vec<String>,
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
enum Call {
    ContentStart { event: ContentStart, ids: OutputIds },
    UserSpeech { start: bool },
    UserTranscript { text: String },
    UserCompleted,
    TextOutput { event: TextOutput, ids: OutputIds },
    AudioOutput { event: AudioOutput, ids: OutputIds },
    Usage { usage: UsageEvent },
}

#[derive(Serialize)]
struct Calls {
    calls: Vec<CallOutput>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum CallOutput {
    ContentStart {
        events: Vec<OutputEvent>,
        response_id: Option<String>,
        item_id: Option<String>,
        conversation_id: Option<String>,
        delta_type: Option<&'static str>,
    },
    Text {
        events: Vec<OutputEvent>,
        delta_chunks: Option<Vec<OutputEvent>>,
    },
    Events {
        events: Vec<OutputEvent>,
    },
    Usage {
        usage: Vec<UsageDelta>,
    },
}

#[derive(Serialize)]
struct UsageDelta {
    input_tokens: i64,
    output_tokens: i64,
    total_tokens: i64,
    input_token_details: TokenDetails,
    output_token_details: TokenDetails,
}

#[derive(Serialize)]
struct TokenDetails {
    audio_tokens: i64,
    text_tokens: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    cached_tokens: Option<i64>,
}

pub(crate) struct Op;

fn failure<E>(error: E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "RealtimeOutput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn token_delta(latest: &UsageTokens, prior: &UsageTokens) -> TokenDetails {
    TokenDetails {
        audio_tokens: latest.speech_tokens.saturating_sub(prior.speech_tokens),
        text_tokens: latest.text_tokens.saturating_sub(prior.text_tokens),
        cached_tokens: None,
    }
}

// LiteLLM reports usage through the next response.done; this reads the same snapshot difference
fn take_usage_delta(session: &mut RealtimeSession) -> Vec<UsageDelta> {
    if session.cumulative_usage == session.reported_usage {
        return Vec::new();
    }
    let latest = &session.cumulative_usage;
    let prior = &session.reported_usage;
    let mut input_token_details =
        token_delta(&latest.details.total.input, &prior.details.total.input);
    input_token_details.cached_tokens = Some(0);
    let delta = UsageDelta {
        input_tokens: latest
            .total_input_tokens
            .saturating_sub(prior.total_input_tokens),
        output_tokens: latest
            .total_output_tokens
            .saturating_sub(prior.total_output_tokens),
        total_tokens: latest.total_tokens.saturating_sub(prior.total_tokens),
        input_token_details,
        output_token_details: token_delta(
            &latest.details.total.output,
            &prior.details.total.output,
        ),
    };
    session.reported_usage = session.cumulative_usage.clone();
    vec![delta]
}

fn content_start_output(
    output: &mut NovaRealtimeOutput<'_>,
    event: &ContentStart,
    ids: &OutputIds,
) -> CallOutput {
    let started = output.transform_content_start_event(event, ids);
    CallOutput::ContentStart {
        events: started.events,
        response_id: started.ids.response,
        item_id: started.ids.item,
        conversation_id: started.ids.conversation,
        delta_type: started.delta_type.map(|kind| match kind {
            DeltaType::Text => "text",
            DeltaType::Audio => "audio",
        }),
    }
}

fn usage_output(output: &mut NovaRealtimeOutput<'_>, usage: &UsageEvent) -> CallOutput {
    output.transform_usage_event(usage.clone());
    CallOutput::Usage {
        usage: take_usage_delta(output.session),
    }
}

fn run_call(
    output: &mut NovaRealtimeOutput<'_>,
    call: &Call,
    chunks: &mut Option<Vec<ResponseDelta>>,
) -> CallOutput {
    match call {
        Call::ContentStart { event, ids } => content_start_output(output, event, ids),
        Call::UserSpeech { start } => CallOutput::Events {
            events: vec![output.transform_user_speech_event(*start)],
        },
        Call::UserTranscript { text } => {
            let transcribed = output.transform_user_transcript_event(text);
            CallOutput::Events {
                events: vec![transcribed],
            }
        }
        Call::UserCompleted => CallOutput::Events {
            events: output
                .user_transcript_completed_events()
                .into_iter()
                .collect(),
        },
        Call::TextOutput { event, ids } => CallOutput::Text {
            events: output
                .transform_text_output_event(event, ids, chunks)
                .into_iter()
                .collect(),
            delta_chunks: chunks.as_ref().map(|kept| {
                kept.iter()
                    .map(|chunk| OutputEvent::TextDelta(chunk.clone()))
                    .collect()
            }),
        },
        Call::AudioOutput { event, ids } => CallOutput::Events {
            events: output
                .transform_audio_output_event(event, ids)
                .into_iter()
                .collect(),
        },
        Call::Usage { usage } => usage_output(output, usage),
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let values = input
            .ids
            .iter()
            .map(String::as_str)
            .map(Uuid::parse_str)
            .collect::<Result<Vec<Uuid>, _>>()
            .map_err(failure)?;
        let source = FixedIds::new(&values);
        let mut session = RealtimeSession::new(&FixedIds::new(&[]));
        let mut output = NovaRealtimeOutput {
            session: &mut session,
            ids: &source,
        };
        let mut chunks: Option<Vec<ResponseDelta>> = None;
        let calls = input
            .calls
            .iter()
            .map(|call| run_call(&mut output, call, &mut chunks))
            .collect();
        serde_json::to_value(Calls { calls }).map_err(failure)
    }
}

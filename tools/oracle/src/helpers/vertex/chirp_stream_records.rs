use betterlite_vertex::chirp_stream::records::{
    ChirpStreamEvent, SpeechActivity, StreamingConfigure, billed_duration, first_request,
    response_event, speech_activity, streaming_config,
};
use google_cloud_speech_v2::model::{
    RecognitionResponseMetadata, SpeechRecognitionAlternative, StreamingRecognitionResult,
    StreamingRecognizeRequest, StreamingRecognizeResponse,
    streaming_recognize_response::SpeechEventType,
};
use google_cloud_wkt::Duration as ProtoDuration;
use jiff::SignedDuration;
use serde::{Deserialize, Serialize};

use crate::helpers::{Helper, HelperError, HelperResult};

mod tests;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    configure: ConfigureInput,
    recognizer: String,
    billed_before: f64,
    responses: Vec<ResponseInput>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigureInput {
    model: String,
    language_codes: Vec<String>,
    sample_rate_hertz: i32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResponseInput {
    speech_event_type: String,
    results: Vec<ResultInput>,
    total_billed_duration: BilledInput,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResultInput {
    alternatives: Vec<AlternativeInput>,
    is_final: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AlternativeInput {
    transcript: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BilledInput {
    seconds: i64,
    nanos: i32,
}

#[derive(Serialize)]
struct Output {
    first_request: FirstRequestOutput,
    events: Vec<ChirpStreamEvent>,
    speech_active: bool,
    billed_seconds: f64,
}

#[derive(Serialize)]
struct FirstRequestOutput {
    recognizer: String,
    has_streaming_config: bool,
    has_audio: bool,
    config: ConfigOutput,
}

#[derive(Serialize)]
struct ConfigOutput {
    model: String,
    language_codes: Vec<String>,
    encoding: String,
    sample_rate_hertz: i32,
    audio_channel_count: i32,
    interim_results: bool,
    enable_voice_activity_events: bool,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let configure = StreamingConfigure {
            model: input.configure.model.clone(),
            language_codes: input.configure.language_codes.clone(),
            sample_rate_hertz: input.configure.sample_rate_hertz,
        };
        let request = first_request(&input.recognizer, streaming_config(&configure));
        let first = project_request(&request)?;
        let mut speech_active = false;
        let mut billed = SignedDuration::ZERO;
        let mut events = Vec::new();
        for response_input in &input.responses {
            let response = sdk_response(response_input)?;
            let activity = speech_activity(&response);
            if activity != SpeechActivity::None {
                speech_active = activity == SpeechActivity::Begin;
            }
            billed = billed.max(billed_duration(&response));
            events.push(response_event(
                &response,
                input.billed_before.mul_add(1.0, billed.as_secs_f64()),
            ));
        }
        let output = Output {
            first_request: first,
            events,
            speech_active,
            billed_seconds: input.billed_before.mul_add(1.0, billed.as_secs_f64()),
        };
        serde_json::to_value(output).map_err(|error| failure(&error.to_string()))
    }
}

fn project_request(request: &StreamingRecognizeRequest) -> Result<FirstRequestOutput, Failure> {
    let config = request
        .streaming_config()
        .ok_or_else(|| failure("the first request must carry a streaming config"))?;
    let body = config
        .config
        .as_ref()
        .ok_or_else(|| failure("the streaming config must carry a recognition config"))?;
    let decoding = body
        .explicit_decoding_config()
        .ok_or_else(|| failure("the recognition config must decode explicitly"))?;
    let features = config
        .streaming_features
        .as_ref()
        .ok_or_else(|| failure("the streaming config must carry streaming features"))?;
    Ok(FirstRequestOutput {
        recognizer: request.recognizer.clone(),
        has_streaming_config: true,
        has_audio: request.audio().is_some(),
        config: ConfigOutput {
            model: body.model.clone(),
            language_codes: body.language_codes.clone(),
            encoding: decoding.encoding.name().unwrap_or_default().to_owned(),
            sample_rate_hertz: decoding.sample_rate_hertz,
            audio_channel_count: decoding.audio_channel_count,
            interim_results: features.interim_results,
            enable_voice_activity_events: features.enable_voice_activity_events,
        },
    })
}

fn sdk_response(input: &ResponseInput) -> Result<StreamingRecognizeResponse, Failure> {
    let results: Vec<StreamingRecognitionResult> = input
        .results
        .iter()
        .map(|result| {
            let alternatives: Vec<SpeechRecognitionAlternative> = result
                .alternatives
                .iter()
                .map(|alternative| {
                    SpeechRecognitionAlternative::new()
                        .set_transcript(alternative.transcript.clone())
                })
                .collect();
            StreamingRecognitionResult::new()
                .set_alternatives(alternatives)
                .set_is_final(result.is_final)
        })
        .collect();
    let billed = ProtoDuration::new(
        input.total_billed_duration.seconds,
        input.total_billed_duration.nanos,
    )
    .map_err(|_source| failure("total_billed_duration is out of range"))?;
    Ok(StreamingRecognizeResponse::new()
        .set_results(results)
        .set_speech_event_type(SpeechEventType::from(input.speech_event_type.as_str()))
        .set_metadata(RecognitionResponseMetadata::new().set_total_billed_duration(billed)))
}

type Failure = Box<HelperError>;

fn failure(message: &str) -> Failure {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}

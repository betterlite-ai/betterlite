use google_cloud_speech_v2::model::{
    ExplicitDecodingConfig, RecognitionConfig, StreamingRecognitionConfig,
    StreamingRecognitionFeatures, StreamingRecognizeRequest, StreamingRecognizeResponse,
    explicit_decoding_config::AudioEncoding,
};
use jiff::SignedDuration;
use serde::Serialize;

mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StreamEventKind {
    Response,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SpeechActivity {
    None,
    Begin,
    End,
}

impl SpeechActivity {
    #[must_use]
    pub fn from_name(name: &str) -> Self {
        match name {
            "SPEECH_ACTIVITY_BEGIN" => Self::Begin,
            "SPEECH_ACTIVITY_END" | "END_OF_SINGLE_UTTERANCE" => Self::End,
            _ => Self::None,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ChirpStreamResult {
    pub transcript: String,
    pub is_final: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct ChirpStreamEvent {
    pub kind: StreamEventKind,
    pub speech_event: SpeechActivity,
    pub results: Vec<ChirpStreamResult>,
    pub billed_seconds: f64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamingConfigure {
    pub model: String,
    pub language_codes: Vec<String>,
    pub sample_rate_hertz: i32,
}

#[must_use]
pub fn streaming_config(configure: &StreamingConfigure) -> StreamingRecognitionConfig {
    let decoding = ExplicitDecodingConfig::new()
        .set_encoding(AudioEncoding::Linear16)
        .set_sample_rate_hertz(configure.sample_rate_hertz)
        .set_audio_channel_count(1_i32);
    let config = RecognitionConfig::new()
        .set_explicit_decoding_config(decoding)
        .set_model(configure.model.clone())
        .set_language_codes(configure.language_codes.clone());
    let features = StreamingRecognitionFeatures::new()
        .set_interim_results(true)
        .set_enable_voice_activity_events(true);
    StreamingRecognitionConfig::new()
        .set_config(config)
        .set_streaming_features(features)
}

#[must_use]
pub fn first_request(
    recognizer: &str,
    config: StreamingRecognitionConfig,
) -> StreamingRecognizeRequest {
    StreamingRecognizeRequest::new()
        .set_recognizer(recognizer)
        .set_streaming_config(config)
}

#[must_use]
pub fn speech_activity(response: &StreamingRecognizeResponse) -> SpeechActivity {
    response
        .speech_event_type
        .name()
        .map_or(SpeechActivity::None, SpeechActivity::from_name)
}

#[must_use]
pub fn billed_duration(response: &StreamingRecognizeResponse) -> SignedDuration {
    let Some(metadata) = &response.metadata else {
        return SignedDuration::ZERO;
    };
    let Some(duration) = &metadata.total_billed_duration else {
        return SignedDuration::ZERO;
    };
    // protobuf caps seconds at 315,576,000,000 and nanos at 999,999,999, so new() cannot overflow
    SignedDuration::new(duration.seconds(), duration.nanos())
}

#[must_use]
pub fn cumulative_billed(billed_before: f64, stream_billed: SignedDuration) -> f64 {
    // float_arithmetic is denied; a fused multiply-add by one is the same sum with the same rounding
    billed_before.mul_add(1.0, stream_billed.as_secs_f64())
}

#[must_use]
pub fn response_event(
    response: &StreamingRecognizeResponse,
    billed_seconds: f64,
) -> ChirpStreamEvent {
    let results = response
        .results
        .iter()
        .map(|result| ChirpStreamResult {
            transcript: result
                .alternatives
                .first()
                .map_or_else(String::new, |alternative| alternative.transcript.clone()),
            is_final: result.is_final,
        })
        .collect();
    ChirpStreamEvent {
        kind: StreamEventKind::Response,
        speech_event: speech_activity(response),
        results,
        billed_seconds,
    }
}

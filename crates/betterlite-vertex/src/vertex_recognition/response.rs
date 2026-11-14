use std::str::FromStr as _;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::decisions::Json;
use betterlite_openai::transcription::duration::duration_number;
use rust_decimal::Decimal;
use serde::Serialize;
use serde_json::Deserializer;

use crate::gemini_response::decode::MAX_RESPONSE_BYTES;
use crate::vertex_recognition::error::VertexRecognitionError;
use crate::vertex_recognition::types::RecognizeResponse;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TranscriptionTask {
    Transcribe,
}

// the transcriber reads text, task, language and duration; LiteLLM keeps the whole provider reply as hidden params
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RecognitionResponse {
    pub text: String,
    pub task: TranscriptionTask,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<Json>,
    #[serde(skip)]
    pub metadata: RecognizeResponse,
}
// LiteLLM's _parse_duration_seconds reads a decimal number with a trailing s and drops everything else
#[must_use]
pub fn parse_duration_seconds(duration: Option<&str>) -> Option<Decimal> {
    let text = duration?.strip_suffix('s')?;
    Decimal::from_str(text)
        .or_else(|_error| Decimal::from_scientific(text))
        .ok()
}
pub fn decode_vertex_recognition(
    reply: &UpstreamResponse,
) -> Result<RecognitionResponse, VertexRecognitionError> {
    if reply.body.len() > MAX_RESPONSE_BYTES {
        return Err(VertexRecognitionError::TooLarge {
            size: reply.body.len(),
        });
    }
    let mut decoder = Deserializer::from_slice(&reply.body);
    let parsed: RecognizeResponse = serde_path_to_error::deserialize(&mut decoder)
        .map_err(|error| decode_error(reply.status, error))?;
    decoder
        .end()
        .map_err(|source| VertexRecognitionError::NotJson {
            status: reply.status,
            source,
        })?;
    Ok(project(parsed))
}
// a syntax failure is the source's non-JSON reply, while a typed field that fails is pydantic's ValidationError
fn decode_error(
    status: u16,
    error: serde_path_to_error::Error<serde_json::Error>,
) -> VertexRecognitionError {
    let path = error.path().to_string();
    let source = error.into_inner();
    if source.is_data() {
        return VertexRecognitionError::Shape { path, source };
    }
    VertexRecognitionError::NotJson { status, source }
}
fn project(parsed: RecognizeResponse) -> RecognitionResponse {
    let transcripts: Vec<&str> = parsed
        .results
        .iter()
        .filter_map(|result| result.alternatives.first()?.transcript.as_deref())
        .filter(|transcript| !transcript.is_empty())
        .collect();
    let language = parsed
        .results
        .iter()
        .find_map(|result| {
            result
                .language_code
                .as_deref()
                .filter(|code| !code.is_empty())
        })
        .map(str::to_owned);
    // every Decimal renders as a JSON number, so a render failure omits the duration like an invalid one
    let duration = parse_duration_seconds(
        parsed
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.total_billed_duration.as_deref()),
    )
    .and_then(|seconds| duration_number(Some(seconds)).ok());
    RecognitionResponse {
        text: transcripts.join(" "),
        task: TranscriptionTask::Transcribe,
        language,
        duration,
        metadata: parsed,
    }
}

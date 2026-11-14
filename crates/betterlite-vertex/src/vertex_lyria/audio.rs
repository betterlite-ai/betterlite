use betterlite_models::metadata::vendor::VertexAudioApi;
use betterlite_streaming::stream_content_assembly::decode_stream_audio_fragment;
use serde::Deserialize;

use crate::gemini_response::decode::MAX_RESPONSE_BYTES;
use crate::model_policy::lyria::LyriaOverlay;
use crate::vertex_speech::audio::BinaryAudio;

use super::error::VertexLyriaError;

// LiteLLM's last resort when neither the provider nor the sniffer names the type
pub const DEFAULT_SPEECH_MEDIA_TYPE: &str = "audio/mpeg";

#[derive(Debug, Clone, Copy)]
pub struct LyriaDecode<'decode> {
    pub model: &'decode str,
    pub status: u16,
    pub body: &'decode [u8],
}

pub fn decode_lyria_audio(
    overlay: &LyriaOverlay<'_>,
    decode: &LyriaDecode<'_>,
    sniff: &dyn Fn(&[u8]) -> Option<String>,
) -> Result<BinaryAudio, VertexLyriaError> {
    let model_info =
        overlay
            .model_info(decode.model)
            .ok_or_else(|| VertexLyriaError::UnknownModel {
                model: decode.model.to_owned(),
            })?;
    decode_lyria_dialect(model_info.vertex_ai_audio_api, decode, sniff)
}

pub fn decode_lyria_dialect(
    api: VertexAudioApi,
    decode: &LyriaDecode<'_>,
    sniff: &dyn Fn(&[u8]) -> Option<String>,
) -> Result<BinaryAudio, VertexLyriaError> {
    if decode.body.len() > MAX_RESPONSE_BYTES {
        return Err(VertexLyriaError::ResponseTooLarge {
            size: decode.body.len(),
            limit: MAX_RESPONSE_BYTES,
        });
    }
    let model = decode
        .model
        .strip_prefix("vertex_ai/")
        .unwrap_or(decode.model);
    let (audio, mime_type) = match api {
        VertexAudioApi::LyriaPredict => predict_audio(decode.body, model)?,
        VertexAudioApi::LyriaInteractions => interactions_audio(decode.body, model)?,
    };
    let bytes = decode_stream_audio_fragment(&audio, MAX_RESPONSE_BYTES)
        .map_err(VertexLyriaError::Audio)?;
    let content_type = mime_type
        .filter(|value| !value.is_empty())
        .or_else(|| sniff(&bytes))
        .unwrap_or_else(|| DEFAULT_SPEECH_MEDIA_TYPE.to_owned());
    Ok(BinaryAudio {
        status: decode.status,
        content_type: Some(content_type),
        bytes,
    })
}

#[derive(Debug, Deserialize)]
struct PredictResponse {
    predictions: Option<Vec<PredictPrediction>>,
}

#[derive(Debug, Deserialize)]
struct PredictPrediction {
    #[serde(rename = "audioContent")]
    audio_content: Option<String>,
    #[serde(rename = "bytesBase64Encoded")]
    bytes_base64_encoded: Option<String>,
    #[serde(rename = "mimeType")]
    mime_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct InteractionResponse {
    steps: Option<Vec<InteractionItem>>,
    outputs: Option<Vec<InteractionItem>>,
}

#[derive(Debug, Deserialize)]
struct InteractionItem {
    #[serde(rename = "type")]
    kind: Option<String>,
    content: Option<Vec<InteractionItem>>,
    data: Option<String>,
    #[serde(rename = "mime_type")]
    mime_type: Option<String>,
}

fn predict_audio(body: &[u8], model: &str) -> Result<(String, Option<String>), VertexLyriaError> {
    let response: PredictResponse =
        serde_json::from_slice(body).map_err(VertexLyriaError::ResponseJson)?;
    let prediction = response
        .predictions
        .as_deref()
        .and_then(|predictions| predictions.first())
        .ok_or_else(|| VertexLyriaError::MissingAudio {
            model: model.to_owned(),
        })?;
    let audio = prediction
        .audio_content
        .as_ref()
        .filter(|value| !value.is_empty())
        .or(prediction.bytes_base64_encoded.as_ref())
        .ok_or_else(|| VertexLyriaError::MissingAudio {
            model: model.to_owned(),
        })?;
    Ok((audio.clone(), prediction.mime_type.clone()))
}

fn interactions_audio(
    body: &[u8],
    model: &str,
) -> Result<(String, Option<String>), VertexLyriaError> {
    let response: InteractionResponse =
        serde_json::from_slice(body).map_err(VertexLyriaError::ResponseJson)?;
    let steps = response.steps.or(response.outputs).unwrap_or_default();
    let mut audio: Option<String> = None;
    let mut mime_type: Option<String> = None;
    for step in &steps {
        let contents: &[InteractionItem] = if step.kind.as_deref() == Some("model_output") {
            step.content.as_deref().unwrap_or_default()
        } else {
            std::slice::from_ref(step)
        };
        for content in contents {
            let data = content.data.as_ref().filter(|value| !value.is_empty());
            if content.kind.as_deref() == Some("audio")
                && let Some(data) = data
            {
                audio = Some(data.clone());
                mime_type = content.mime_type.clone();
            }
        }
    }
    match audio {
        Some(audio) => Ok((audio, mime_type)),
        None => Err(VertexLyriaError::MissingAudio {
            model: model.to_owned(),
        }),
    }
}

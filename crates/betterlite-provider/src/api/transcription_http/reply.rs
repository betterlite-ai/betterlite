use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::call_helpers::headers::{
    AdditionalHeaders, ProviderHeaderProjection, set_provider_response_headers,
};
use betterlite_openai::api::audio_subtitles::SUBTITLE_RESPONSE_FORMATS;
use betterlite_openai::api::audio_subtitles::timing::synthesize_subtitle_document;
use betterlite_openai::api::transcription_contract::TranscriptionResponse;
use betterlite_openai::files::stream::flatten_headers;
use betterlite_openai::openai_base_config::Object;

use super::config::TranscriptionHttpConfig;
use super::error::TranscriptionHttpError;

#[derive(Debug)]
pub struct TranscriptionReply {
    pub response: TranscriptionResponse,
    pub provider_headers: ProviderHeaderProjection,
}

// _transform_audio_transcription_response: the provider's record, then the hidden headers, then the subtitle rewrite
pub(super) fn transform_response<C>(
    config: &C,
    optional_params: &Object,
    answer: &UpstreamResponse,
) -> Result<TranscriptionReply, TranscriptionHttpError>
where
    C: TranscriptionHttpConfig,
{
    let mut transformed = config.transform_audio_response(answer)?;
    let provider_headers = set_provider_response_headers(
        &AdditionalHeaders::default(),
        &flatten_headers(&answer.headers),
    );
    let subtitle = match optional_params.get("response_format") {
        Some(serde_json::Value::String(format))
            if SUBTITLE_RESPONSE_FORMATS.contains(&format.as_str()) =>
        {
            Some(format.as_str())
        }
        Some(_) | None => None,
    };
    if config.supports_subtitle_synthesis()
        && let Some(format) = subtitle
    {
        let absent = serde_json::Value::Null;
        let words = transformed.extra.get("words").unwrap_or(&absent);
        if let Some(document) = synthesize_subtitle_document(words, format)? {
            transformed.text = Some(document);
        }
        // "words" leaves the record even when no document could be built
        let _removed = transformed.extra.remove("words");
    }
    Ok(TranscriptionReply {
        response: transformed,
        provider_headers,
    })
}

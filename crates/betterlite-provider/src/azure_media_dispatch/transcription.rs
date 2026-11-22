use std::io::{self, Read};
use std::path::Path;

use betterlite_azure::credentials::cache::RefreshProviderCache;
use betterlite_azure::transcription::call::audio_transcriptions;
use betterlite_azure::transcription::call::client::ClientWiring;
use betterlite_azure::transcription::call::logging::AzureTranscriptionLogger;
use betterlite_azure::transcription::call::reply::finish_transcription;
use betterlite_azure::transcription::call::{AzureTranscriptionCall, AzureTranscriptionError};
use betterlite_azure::transcription::native::reply::{
    NativeTranscriptionReply, decode_native_transcription,
};
use betterlite_azure::transcription::native::{
    complete_url, get_error_class, transform_request, validate_environment,
};
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_openai::api::audio_file_input::{
    AudioFile, AudioTuple, AuthorizedAudioFiles, TupleContent,
};
use betterlite_openai::api::transcription_contract::AudioTranscriptionData;
use betterlite_openai::files::stream::flatten_headers;
use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::transcription::reply::TranscriptionReply;
use betterlite_openai::transcription::transport::TranscriptionTransport;
use http::Method;
use http::header::{CONTENT_TYPE, HeaderValue};
use url::Url;

use super::call::{NativeTranscriptionOperation, TranscriptionOperation};
use super::error::AzureMediaError;
use super::parts::{header_map, lossy_text};
use super::runtime::MediaRuntime;

const DEFAULT_CONTENT_TYPE: &str = "audio/wav";

// AzureAudioTranscription.audio_transcriptions: select the sync or async client, send, then decode and log the reply
pub(super) async fn openai<C, K, T, L>(
    operation: TranscriptionOperation,
    runtime: &mut MediaRuntime<'_, C, K, T, L>,
) -> Result<TranscriptionReply, AzureMediaError>
where
    T: TranscriptionTransport,
    L: AzureTranscriptionLogger,
{
    let call = AzureTranscriptionCall {
        input: operation.input,
        file: &operation.file,
        boundary: operation.boundary,
        model: &operation.model,
        optional_params: operation.optional_params,
        atranscription: operation.atranscription,
        api_version: operation.api_version,
        api_key: operation.api_key,
        api_base: operation.api_base,
        azure_ad_token: operation.azure_ad_token,
        max_retries: operation.max_retries,
        timeout: operation.timeout,
        params: operation.params,
        provider: operation.provider,
        supplied: operation.supplied,
        session: operation.session,
        refresh_enabled: operation.refresh_enabled,
    };
    exchange_call(&call, runtime).await
}

async fn exchange_call<C, K, T, L>(
    call: &AzureTranscriptionCall<'_>,
    runtime: &mut MediaRuntime<'_, C, K, T, L>,
) -> Result<TranscriptionReply, AzureMediaError>
where
    T: TranscriptionTransport,
    L: AzureTranscriptionLogger,
{
    let mut refresh = RefreshProviderCache::new(runtime.sources);
    let mut wiring = ClientWiring {
        settings: runtime.endpoints,
        environment: runtime.environment,
        sources: runtime.sources,
        cache: &mut *runtime.cache,
        refresh: &mut refresh,
    };
    let (outbound, pending) = audio_transcriptions(call, &mut wiring, &mut *runtime.logger)?;
    let sent = runtime.transport.send(outbound).await;
    Ok(finish_transcription(pending, sent, &mut *runtime.logger)?)
}

// AzureSpeechAudioTranscriptionConfig: validate the Speech auth, then send the raw audio to the short-audio endpoint.
pub(super) async fn native<C, K, T, L>(
    operation: &NativeTranscriptionOperation,
    runtime: &MediaRuntime<'_, C, K, T, L>,
) -> Result<NativeTranscriptionReply, AzureMediaError>
where
    T: TranscriptionTransport,
{
    let outbound = native_request(operation)?;
    let sent = runtime
        .transport
        .send(outbound)
        .await
        .map_err(|source| AzureTranscriptionError::Transport { source })?;
    let response_headers: Headers = flatten_headers(&sent.headers).into_iter().collect();
    if !(200..300).contains(&sent.status) {
        // _handle_error hands the provider config the response text, status and headers
        return Err(AzureMediaError::Native(get_error_class(
            lossy_text(&sent.body),
            sent.status,
            response_headers,
        )));
    }
    Ok(decode_native_transcription(
        sent.status,
        &response_headers,
        &sent.body,
    )?)
}

fn native_request(
    operation: &NativeTranscriptionOperation,
) -> Result<OutboundRequest, AzureMediaError> {
    let validated = validate_environment(
        &operation.headers,
        operation.api_key.as_ref(),
        operation.environment_key.as_ref(),
    )?;
    let endpoint = complete_url(
        operation.api_base.as_deref(),
        operation.environment_base.as_deref(),
        &operation.optional_params,
    )?;
    let audio = AudioFile::Tuple(AudioTuple {
        filename: &operation.filename,
        content: Some(TupleContent::Bytes(&operation.content)),
    });
    let transformed = transform_request(audio, &NoFiles)?;
    let payload = native_bytes(transformed.data)?;
    // the provider config's content type replaces the validated one, as the HTTP handler does after validate_environment
    let content_type = content_type_header(transformed.content_type)?;
    let mut request_headers = header_map(&validated)?;
    request_headers.insert(CONTENT_TYPE, content_type.clone());
    Ok(OutboundRequest {
        method: Method::POST,
        url: Url::parse(&endpoint).map_err(AzureMediaError::Url)?,
        query: Vec::new(),
        headers: request_headers,
        body: RequestBody::Bytes {
            content_type,
            data: payload,
        },
        deadline: operation.timeout,
    })
}

fn content_type_header(text: Option<String>) -> Result<HeaderValue, AzureMediaError> {
    let content_type_text = text.unwrap_or_else(|| String::from(DEFAULT_CONTENT_TYPE));
    HeaderValue::from_str(&content_type_text).map_err(|source| AzureMediaError::HeaderValue {
        name: String::from("Content-Type"),
        source,
    })
}

// the native transform always produces the audio bytes; the other variant cannot come out of process_audio_file
pub(super) fn native_bytes(data: AudioTranscriptionData) -> Result<Vec<u8>, AzureMediaError> {
    match data {
        AudioTranscriptionData::Bytes(bytes) => Ok(bytes),
        AudioTranscriptionData::Object(_) => Err(AzureMediaError::NativeRequest),
    }
}

// the native transform reads no path, and a path arriving here is a caller error rather than a filesystem read
pub(super) struct NoFiles;

impl AuthorizedAudioFiles for NoFiles {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            path.display().to_string(),
        ))
    }

    fn open(&self, path: &Path) -> io::Result<Box<dyn Read + Send>> {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            path.display().to_string(),
        ))
    }
}

use betterlite_azure::credentials::cache::RefreshProviderCache;
use betterlite_azure::native_speech::response::HttpxBinaryResponseContent;
use betterlite_azure::speech::binary::{AudioSpeechCall, audio_speech, finish_audio_speech};
use betterlite_azure::speech::client::ClientWiring;
use betterlite_azure::speech::error::AzureSpeechError;
use betterlite_logging::logging_pre_post::context::LogContext;
use betterlite_openai::transcription::transport::TranscriptionTransport;

use super::super::call::OpenAiSpeechOperation;
use super::super::error::AzureMediaError;
use super::super::runtime::MediaRuntime;

// AzureChatCompletion.audio_speech's binary path: build the SDK-equivalent post, send it once, then read the bytes.
pub(crate) async fn openai<C, K, T, L>(
    operation: OpenAiSpeechOperation,
    runtime: &mut MediaRuntime<'_, C, K, T, L>,
) -> Result<HttpxBinaryResponseContent, AzureMediaError>
where
    C: Sync,
    K: Sync,
    T: TranscriptionTransport,
    L: Send,
{
    let call = AudioSpeechCall {
        model: &operation.model,
        input: &operation.input,
        voice: &operation.voice,
        optional_params: operation.optional_params,
        aspeech: operation.aspeech,
        api_key: operation.api_key,
        api_base: operation.api_base,
        api_version: operation.api_version,
        params: operation.params,
        provider: operation.provider,
        supplied: operation.supplied,
        session: operation.session,
        refresh_enabled: operation.refresh_enabled,
    };
    let outbound = {
        // the refresh cache holds credential sources the future cannot carry across the send
        let mut refresh = RefreshProviderCache::new(runtime.sources);
        let mut wiring = ClientWiring {
            settings: runtime.endpoints,
            environment: runtime.environment,
            sources: runtime.sources,
            cache: &mut *runtime.cache,
            refresh: &mut refresh,
        };
        let mut logger = LogContext {
            state: &mut *runtime.speech_state,
            env: runtime.speech_env,
        };
        audio_speech(&call, &mut wiring, &mut logger)?
    };
    let sent = runtime
        .transport
        .send(outbound)
        .await
        .map_err(AzureSpeechError::from)?;
    Ok(finish_audio_speech(sent)?)
}

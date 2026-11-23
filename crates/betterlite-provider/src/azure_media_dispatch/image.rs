use betterlite_azure::image_transport::ImageClock;
use betterlite_azure::image_transport::request::ImageClientConstructor;
use betterlite_azure::images::generation::{
    GeneratedImage, Generation, Runtime as GenerationRuntime, begin, finish, send,
};
use betterlite_openai::image_generation::logging::ImageGenerationLogger;

use super::call::ImageGenerationOperation;
use super::error::AzureMediaError;
use super::runtime::MediaRuntime;

// AzureChatCompletion.image_generation: begin runs the pre_call, send crosses the transport and finish logs and decodes the reply
pub(super) async fn generation<C, K, T, L>(
    operation: ImageGenerationOperation,
    runtime: &mut MediaRuntime<'_, C, K, T, L>,
) -> Result<GeneratedImage, AzureMediaError>
where
    C: ImageClientConstructor,
    K: ImageClock,
    T: Sync,
    L: ImageGenerationLogger + Send,
{
    let call = Generation {
        model: operation.model.as_deref(),
        prompt: &operation.prompt,
        timeout: operation.timeout,
        optional_params: operation.optional_params,
        headers: operation.headers,
        api_key: operation.api_key.as_ref(),
        api_base: operation.api_base.as_deref(),
        api_version: operation.api_version.as_deref(),
        azure_ad_token: operation.azure_ad_token.as_ref(),
        azure_ad_token_provider: operation.provider,
        base_model: operation.base_model.as_deref(),
        params: operation.params,
        delegate_async: operation.delegate_async,
        now_seconds: operation.now_seconds,
    };
    let begun = begin(call, &mut generation_runtime(runtime))?;
    let sent = send(
        &begun,
        runtime.constructor,
        runtime.clock,
        runtime.polling_timeout,
    )
    .await;
    Ok(finish(&begun, sent, &mut *runtime.logger)?)
}

// the leaf's runtime borrows the logger from the composition's runtime for the length of one begin
fn generation_runtime<'run, C, K, T, L>(
    runtime: &'run mut MediaRuntime<'_, C, K, T, L>,
) -> GenerationRuntime<'run, C, K>
where
    L: ImageGenerationLogger,
{
    GenerationRuntime {
        endpoints: runtime.endpoints,
        environment: runtime.environment,
        sources: runtime.sources,
        refresh_enabled: runtime.refresh_enabled,
        polling_timeout: runtime.polling_timeout,
        constructor: runtime.constructor,
        clock: runtime.clock,
        logger: &mut *runtime.logger,
    }
}

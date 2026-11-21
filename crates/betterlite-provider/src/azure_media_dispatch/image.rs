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
    L: ImageGenerationLogger,
{
    let ImageGenerationOperation {
        model,
        prompt,
        timeout,
        optional_params,
        headers,
        api_key,
        api_base,
        api_version,
        azure_ad_token,
        provider,
        base_model,
        params,
        delegate_async,
        now_seconds,
    } = operation;
    let call = Generation {
        model: model.as_deref(),
        prompt: &prompt,
        timeout,
        optional_params,
        headers,
        api_key: api_key.as_ref(),
        api_base: api_base.as_deref(),
        api_version: api_version.as_deref(),
        azure_ad_token: azure_ad_token.as_ref(),
        azure_ad_token_provider: provider,
        base_model: base_model.as_deref(),
        params,
        delegate_async,
        now_seconds,
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

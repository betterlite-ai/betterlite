use betterlite_core::api::image_generation_call::operations::{BranchAnswer, FoundryRequest};
use betterlite_error::dispatch::RawException;
use betterlite_http::proxy::ProxiedClient;

use super::{Post, answer, model_prompt_body};

const FOUNDRY_DEFAULT_VERSION: &str = "2025-02-01-preview";

pub(in crate::sdk_http::image_generation_call) async fn send(
    request: FoundryRequest<'_>,
    client: &ProxiedClient,
) -> Result<BranchAnswer, Box<RawException>> {
    let model = request.model.unwrap_or_default();
    let base = request.api_base.unwrap_or_default().trim_end_matches('/');
    let version = request.api_version.unwrap_or(FOUNDRY_DEFAULT_VERSION);
    let url = format!("{base}/openai/deployments/{model}/images/generations?api-version={version}");
    let body = model_prompt_body(
        request.base_model.unwrap_or(model),
        request.prompt,
        request.optional_params,
    );
    answer(
        client,
        Post {
            url,
            headers: request.headers,
            body,
            timeout: request.timeout,
            model,
            provider: "azure_ai",
            exception_type: "AzureOpenAIError",
        },
    )
    .await
}

use betterlite_azure::credentials::source::TokenReply;
use betterlite_core::api::image_generation_call::operations::{
    AzureRequest, BranchAnswer, Headers,
};
use betterlite_error::dispatch::RawException;
use betterlite_http::proxy::ProxiedClient;

use super::{Post, answer, has_auth, inserts_bearer, model_prompt_body, plain};

pub(in crate::sdk_http::image_generation_call) async fn azure(
    request: AzureRequest<'_>,
    client: &ProxiedClient,
) -> Result<BranchAnswer, Box<RawException>> {
    let model = request.model.unwrap_or_default();
    let base = request.api_base.unwrap_or_default().trim_end_matches('/');
    let version = request.api_version.unwrap_or_default();
    let url = format!("{base}/openai/deployments/{model}/images/generations?api-version={version}");
    let mut headers = request.headers.clone();
    if request.api_key.is_none() && !has_auth(&headers) {
        token_headers(&request, model, &mut headers)?;
    }
    let body = model_prompt_body(
        request.base_model.unwrap_or(model),
        request.prompt,
        request.optional_params,
    );
    answer(
        client,
        Post {
            url,
            headers: &headers,
            body,
            timeout: request.timeout,
            model,
            provider: "azure",
            exception_type: "AzureOpenAIError",
        },
    )
    .await
}

// Only an absent api_key leaves room for the token; a key already put api-key in the header map
fn token_headers(
    request: &AzureRequest<'_>,
    model: &str,
    headers: &mut Headers,
) -> Result<(), Box<RawException>> {
    if let Some(token) = request.azure_ad_token {
        inserts_bearer(headers, token);
        return Ok(());
    }
    let Some(provider) = request.azure_ad_token_provider else {
        return Ok(());
    };
    match provider.token() {
        Ok(TokenReply::Text(issued)) => {
            inserts_bearer(headers, &issued);
            Ok(())
        }
        Ok(TokenReply::NonText { type_name }) => Err(plain(
            &format!("the azure token provider answered a {type_name}"),
            model,
            "azure",
        )),
        Err(failure) => Err(plain(&failure.message, model, "azure")),
    }
}

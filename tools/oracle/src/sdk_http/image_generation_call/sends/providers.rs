use betterlite_azure::credentials::source::TokenReply;
use betterlite_core::api::image_generation_call::operations::{
    AzureRequest, BranchAnswer, FoundryRequest, GenericRequest, Headers,
};
use betterlite_error::dispatch::RawException;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::decisions::Json;
use secrecy::ExposeSecret as _;

use super::{
    Post, has_auth, inserts_bearer, model_prompt_body, parsed_response, plain, post, prompt_body,
};

const FOUNDRY_DEFAULT_VERSION: &str = "2025-02-01-preview";

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
    let body = prompt_body(request.prompt, request.optional_params);
    let response = post(
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
    .await?;
    Ok(BranchAnswer {
        response: parsed_response(&response.body, model, "azure")?,
        hidden_model: None,
    })
}

// Only an absent api_key leaves room for the token; a key already put api-key in the header map
fn token_headers(
    request: &AzureRequest<'_>,
    model: &str,
    headers: &mut Headers,
) -> Result<(), Box<RawException>> {
    match request.azure_ad_token {
        Some(token) => inserts_bearer(headers, token),
        None => match request.azure_ad_token_provider {
            Some(provider) => match provider.token() {
                Ok(TokenReply::Text(token)) => inserts_bearer(headers, &token),
                Ok(TokenReply::NonText { type_name }) => {
                    return Err(plain(
                        &format!("the azure token provider answered a {type_name}"),
                        model,
                        "azure",
                    ));
                }
                Err(failure) => {
                    return Err(plain(&failure.message, model, "azure"));
                }
            },
            None => {}
        },
    }
    Ok(())
}

pub(in crate::sdk_http::image_generation_call) async fn foundry(
    request: FoundryRequest<'_>,
    client: &ProxiedClient,
) -> Result<BranchAnswer, Box<RawException>> {
    let model = request.model.unwrap_or_default();
    let base = request.api_base.unwrap_or_default().trim_end_matches('/');
    let version = request.api_version.unwrap_or(FOUNDRY_DEFAULT_VERSION);
    let url = format!("{base}/openai/deployments/{model}/images/generations?api-version={version}");
    let body = prompt_body(request.prompt, request.optional_params);
    let response = post(
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
    .await?;
    Ok(BranchAnswer {
        response: parsed_response(&response.body, model, "azure_ai")?,
        hidden_model: None,
    })
}

pub(in crate::sdk_http::image_generation_call) async fn generic(
    request: GenericRequest<'_>,
    client: &ProxiedClient,
) -> Result<BranchAnswer, Box<RawException>> {
    let model = request.model.unwrap_or_default();
    let base = match request.litellm_params.get("api_base") {
        Some(Json::Text(text)) => text.clone(),
        None | Some(Json::Null) => {
            return Err(plain(
                "No api_base provided for the image generation request",
                model,
                request.provider,
            ));
        }
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(plain("api_base must be a string", model, request.provider));
        }
    };
    let Some(key) = request.api_key else {
        return Err(plain(
            &format!("{}_API_KEY is not set", request.provider.to_uppercase()),
            model,
            request.provider,
        ));
    };
    let mut headers = Headers::new();
    headers.insert(
        String::from("Content-Type"),
        String::from("application/json"),
    );
    headers.insert(
        String::from("Authorization"),
        format!("Bearer {}", key.expose_secret()),
    );
    let body = model_prompt_body(model, request.prompt, request.optional_params);
    let response = post(
        client,
        Post {
            url: format!("{}/v1/images/generations", base.trim_end_matches('/')),
            headers: &headers,
            body,
            timeout: request.timeout,
            model,
            provider: request.provider,
            exception_type: "BaseLLMException",
        },
    )
    .await?;
    Ok(BranchAnswer {
        response: parsed_response(&response.body, model, request.provider)?,
        hidden_model: None,
    })
}

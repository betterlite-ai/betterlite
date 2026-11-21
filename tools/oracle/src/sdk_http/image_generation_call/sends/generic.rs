use betterlite_core::api::image_generation_call::operations::{
    BranchAnswer, GenericRequest, Headers,
};
use betterlite_error::dispatch::RawException;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::decisions::Json;
use secrecy::ExposeSecret as _;

use super::{Post, answer, model_prompt_body, plain};

pub(in crate::sdk_http::image_generation_call) async fn send(
    request: GenericRequest<'_>,
    client: &ProxiedClient,
) -> Result<BranchAnswer, Box<RawException>> {
    let resolved_model = request.model.unwrap_or_default();
    let api_base = match request.litellm_params.get("api_base") {
        Some(Json::Text(text)) => text.clone(),
        None | Some(Json::Null) => {
            return Err(plain(
                "No api_base provided for the image generation request",
                resolved_model,
                request.provider,
            ));
        }
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(plain(
                "api_base must be a string",
                resolved_model,
                request.provider,
            ));
        }
    };
    let Some(key) = request.api_key else {
        return Err(plain(
            &format!("{}_API_KEY is not set", request.provider.to_uppercase()),
            resolved_model,
            request.provider,
        ));
    };
    let mut authorization = Headers::new();
    authorization.insert(
        String::from("Content-Type"),
        String::from("application/json"),
    );
    authorization.insert(
        String::from("Authorization"),
        format!("Bearer {}", key.expose_secret()),
    );
    let payload = model_prompt_body(resolved_model, request.prompt, request.optional_params);
    answer(
        client,
        Post {
            url: format!("{}/v1/images/generations", api_base.trim_end_matches('/')),
            headers: &authorization,
            body: payload,
            timeout: request.timeout,
            model: resolved_model,
            provider: request.provider,
            exception_type: "BaseLLMException",
        },
    )
    .await
}

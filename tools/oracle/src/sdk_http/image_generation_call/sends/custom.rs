use std::future::Future;
use std::pin::Pin;

use betterlite_core::api::image_generation_call::operations::{
    BranchAnswer, CustomImageHandler, CustomRequest, Headers,
};
use betterlite_error::dispatch::RawException;
use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::{Post, parsed_response, post};

pub(in crate::sdk_http::image_generation_call) struct CustomImage {
    pub(in crate::sdk_http::image_generation_call) source: ProxiedClient,
}

impl CustomImageHandler for CustomImage {
    fn generate<'handler>(
        &'handler self,
        request: CustomRequest<'handler>,
        _logging: &'handler mut LoggingState,
    ) -> Pin<Box<dyn Future<Output = Result<BranchAnswer, Box<RawException>>> + Send + 'handler>>
    {
        let source = self.source.clone();
        Box::pin(async move { send(request, &source).await })
    }
}

pub(in crate::sdk_http::image_generation_call) async fn send(
    request: CustomRequest<'_>,
    client: &ProxiedClient,
) -> Result<BranchAnswer, Box<RawException>> {
    let model = request.model.unwrap_or_default();
    let base = request.api_base.unwrap_or_default().trim_end_matches('/');
    let mut headers = Headers::new();
    headers.insert(
        String::from("Content-Type"),
        String::from("application/json"),
    );
    let mut body = IndexMap::new();
    body.insert(String::from("model"), Json::Text(model.to_owned()));
    body.insert(
        String::from("prompt"),
        Json::Text(request.prompt.to_owned()),
    );
    let response = post(
        client,
        Post {
            url: format!("{base}/custom/images"),
            headers: &headers,
            body: Json::Object(body),
            timeout: request.timeout,
            model,
            provider: request.provider,
            exception_type: "RateLimitError",
        },
    )
    .await?;
    Ok(BranchAnswer {
        response: parsed_response(&response.body, model, request.provider)?,
        hidden_model: None,
    })
}

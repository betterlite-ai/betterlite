use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::ResponseApiUsage;
use std::collections::BTreeMap;
use std::error::Error;

pub struct EstimateRequest<'request> {
    pub model: &'request str,
    pub request_input: &'request Json,
    pub responses_api_request: &'request Json,
    pub generated_text: &'request str,
}

pub struct TokenCountRequest<'request> {
    pub model: &'request str,
    pub messages: Option<&'request [Json]>,
    pub text: Option<&'request str>,
    pub count_response_tokens: bool,
}

pub trait UsageTokenization {
    type Error: Error + Send + Sync + 'static;

    fn messages(&self, request: &EstimateRequest<'_>) -> Result<Vec<Json>, Self::Error>;

    fn count(&self, request: TokenCountRequest<'_>) -> Result<i64, Self::Error>;
}

pub fn estimate_usage_from_text<T>(
    tokenization: &T,
    request: &EstimateRequest<'_>,
) -> Result<ResponseApiUsage, T::Error>
where
    T: UsageTokenization,
{
    let messages = tokenization.messages(request)?;
    let input_tokens = tokenization.count(TokenCountRequest {
        model: request.model,
        messages: Some(&messages),
        text: None,
        count_response_tokens: false,
    })?;
    let output_tokens = tokenization.count(TokenCountRequest {
        model: request.model,
        messages: None,
        text: Some(request.generated_text),
        count_response_tokens: true,
    })?;
    Ok(ResponseApiUsage {
        input_tokens,
        input_tokens_details: None,
        output_tokens,
        output_tokens_details: None,
        total_tokens: input_tokens.saturating_add(output_tokens),
        cost: None,
        extra: BTreeMap::new(),
    })
}

pub fn estimate_usage_safely<T>(
    tokenization: &T,
    request: &EstimateRequest<'_>,
) -> Option<ResponseApiUsage>
where
    T: UsageTokenization,
{
    estimate_usage_from_text(tokenization, request)
        .inspect_err(|error| {
            tracing::debug!("Could not estimate usage from stream text, billing $0: {error}");
        })
        .ok()
}

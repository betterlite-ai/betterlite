use betterlite_azure::completion::arguments::{CompletionCall, SentCall};
use betterlite_azure::completion::error::AzureTextError;
use betterlite_azure::completion::handler::AzureTextCompletion;
use betterlite_azure::completion::request::Retries;
use betterlite_azure::endpoints::cloudflare::CloudflareParams;
use betterlite_azure::endpoints::projection::LiteLlmParams;
use betterlite_models::decisions::{Extra, Json};
use betterlite_oracle_helper_api::error::HelperError;
use secrecy::SecretString;

use super::super::CallCase;
use super::super::hooks::Recorder;
use super::super::view;
use super::{Fixed, block_on, collect, error_report, runtime, source};

pub(super) fn direct_stream(case: &CallCase) -> Result<Json, Box<HelperError>> {
    let source = source(case);
    let handle = source.handle();
    let mut params = Extra::new();
    let data = case.data.clone();
    let prompt = match data.get("prompt") {
        Some(Json::Text(text)) => text.clone(),
        Some(_other) | None => String::new(),
    };
    let litellm = LiteLlmParams::default();
    let call = CompletionCall {
        model: case.model.as_deref(),
        messages: None,
        api_key: case
            .api_key
            .as_ref()
            .map(|value| SecretString::from(value.clone())),
        api_base: case.api_base.as_deref(),
        api_version: case.api_version.as_deref(),
        azure_ad_token: case
            .azure_ad_token
            .as_ref()
            .map(|value| SecretString::from(value.clone())),
        azure_ad_token_provider: None,
        timeout: case.timeout,
        optional_params: &mut params,
        litellm_params: litellm.clone(),
        cloudflare_params: CloudflareParams::default(),
        acompletion: case.acompletion,
        headers: None,
        supplied: None,
    };
    let mut sent = SentCall {
        model: case.model.clone().unwrap_or_default(),
        data,
        prompt,
        max_retries: Retries::pop(&mut Extra::new()),
        litellm_params: &litellm,
        supplied: Some(handle),
    };
    let fixed = Fixed::new();
    let mut recorder = Recorder::default();
    let handler = AzureTextCompletion::new(source.clone());
    let stream = if case.acompletion {
        match block_on(handler.async_streaming(&sent, &call, &runtime(&fixed), &mut recorder))? {
            Ok(stream) => stream,
            Err(error) => return Ok(error_report(&source, &recorder, &error)),
        }
    } else {
        match block_on(handler.streaming(&mut sent, &call, &runtime(&fixed), &mut recorder))? {
            Ok(stream) => stream,
            Err(failure) => {
                let error: AzureTextError = failure.into_azure();
                return Ok(error_report(&source, &recorder, &error));
            }
        }
    };
    let chunks = collect(stream)?;
    Ok(view::report(
        &source,
        &recorder,
        view::Outcome {
            name: "stream",
            reply: Json::Null,
            chunks: Json::List(chunks),
            error: Json::Null,
        },
    ))
}

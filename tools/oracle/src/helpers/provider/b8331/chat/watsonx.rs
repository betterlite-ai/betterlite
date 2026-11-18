use betterlite_openai::openai_base_config::Object as JsonMap;
use betterlite_openai::watsonx_audio::error::api_status;
use betterlite_openai::watsonx_auth::types::WatsonxParams;
use betterlite_openai::watsonx_chat::params::map_openai_params as map_chat_params;
use betterlite_openai::watsonx_chat::plan::{WatsonxChatPlan, WatsonxChatRequest};
use betterlite_prompts::arguments::json::Object;
use betterlite_provider::data_cloud_dispatch::context::{DispatchContext, DispatchGlobals};
use betterlite_provider::enterprise_ml_dispatch::call::{SagemakerRequest, convert};
use betterlite_provider::enterprise_ml_dispatch::context::ChatContext;
use betterlite_provider::enterprise_ml_dispatch::watsonx;
use betterlite_provider::watsonx_text::WatsonxTextConfig;
use betterlite_provider::watsonx_text::request::{
    complete_url, map_openai_params as map_text_params, map_special_auth_params,
};
use chrono::DateTime;
use http::HeaderMap;
use serde_json::Value;

use super::super::case::{Case, chat_body, chat_options};
use super::super::fixture::Fixture;
use super::super::wire::{Failure, failure};
use super::{chat_failure, watsonx_auth};

// _complete_watsonx: B5971's plan over the IAM token, then the OpenAILike body
pub(in crate::b8331) async fn watsonx_chat(case: &Case, fixture: &Fixture) -> Result<(), Failure> {
    let mut mapped = JsonMap::new();
    map_chat_params(
        &chat_options(&case.non_default(), case.stream.unwrap_or(false)),
        &mut mapped,
    )
    .map_err(chat_failure)?;
    let options: WatsonxParams = mapped.into_iter().collect();
    let api_base = case.api_base();
    let api_key = case.key();
    let auth = watsonx_auth(fixture)?;
    let plan = WatsonxChatPlan::prepare(
        &auth,
        WatsonxChatRequest {
            model: &case.model,
            api_base: api_base.as_deref(),
            api_key: api_key.as_ref(),
            headers: HeaderMap::new(),
            options,
        },
    )
    .await
    .map_err(chat_failure)?;
    let messages = serde_json::to_value(&case.messages).map_err(|error| failure(None, &error))?;
    let body = chat_body(plan.model, messages, plan.options);
    fixture.transport.keep_json(plan.url, plan.headers, &body)
}

// _complete_watsonx_text: the key, base and credentials chains, then B5972's generation body
pub(in crate::b8331) async fn watsonx_text(case: &Case, fixture: &Fixture) -> Result<(), Failure> {
    let params = case.credentials_params();
    let litellm_params = Object::new();
    let api_base = case.api_base();
    let api_key = case.key();
    let base = DispatchContext {
        acompletion: false,
        api_base: api_base.as_deref(),
        api_key: api_key.as_ref(),
        custom_llm_provider: "watsonx_text",
        headers: None,
        litellm_params: &litellm_params,
        messages: &[],
        model: &case.model,
        optional_params: &params,
        provider_config: None,
        stream: case.stream,
        timeout: None,
        globals: DispatchGlobals::default(),
        env: fixture.source(),
    };
    let chat = ChatContext {
        base,
        sagemaker: SagemakerRequest::default(),
    };
    let call = watsonx::text_call(&chat).map_err(|error| failure(error.status(), &error))?;
    let mut options: WatsonxParams =
        convert(&call.optional_params).map_err(|error| failure(error.status(), &error))?;
    let non_default = case.non_default();
    map_text_params(&non_default, &mut options);
    map_special_auth_params(&non_default, &mut options);
    let auth = watsonx_auth(fixture)?;
    let headers = auth
        .validate_environment(HeaderMap::new(), &mut options, call.api_key.as_ref())
        .await
        .map_err(|error| failure(api_status(&error), &error))?;
    let stream = matches!(options.get("stream"), Some(Value::Bool(true)));
    let url = complete_url(
        &auth.environment,
        call.api_base.as_deref(),
        &call.model,
        &mut options,
        stream,
    )
    .map_err(|error| failure(error.status(), &error))?;
    let config = WatsonxTextConfig {
        environment: &auth.environment,
        prompts: &fixture.prompts,
        now: DateTime::from_timestamp(1_700_000_000, 0).unwrap_or_default(),
    };
    let body = config
        .transform_request(&call.model, &case.messages, options)
        .await
        .map_err(|error| failure(error.status(), &error))?;
    fixture.transport.keep_json(url, headers, &body)
}

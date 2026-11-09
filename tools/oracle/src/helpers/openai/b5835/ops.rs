use std::collections::{BTreeMap, BTreeSet};

use betterlite_openai::openai_base_config::{
    CatalogCapabilities, ConfigFamily, Headers, OpenAiConfig, OpenAiSettings,
};
use betterlite_openai::vercel_ai_gateway::VercelAiGatewayEnvironment;
use betterlite_openai::vercel_ai_gateway::chat::VercelAiGatewayConfig;
use betterlite_openai::vercel_ai_gateway::embedding::{
    SUPPORTED_OPENAI_PARAMS, get_complete_url, map_openai_params as map_embedding_params,
    transform_embedding_request, validate_environment,
};
use betterlite_openai::vercel_ai_gateway::models::{models_url, parse_models};
use betterlite_openai::vercel_ai_gateway::provider_info::provider_info;
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};

use super::input::{HeadersStep, Input, MapStep, ModelsStep, ProviderStep, RequestStep, Step};

pub(crate) fn run(input: &Input) -> Value {
    let mut results = Vec::with_capacity(input.steps.len());
    for step in &input.steps {
        results.push(execute(step));
    }
    Value::Array(results)
}

fn execute(step: &Step) -> Value {
    match step {
        Step::ProviderInfoAndSupportedParams(input) => provider_step(input),
        Step::MapParams(input) => map_step(input),
        Step::EmbeddingHeadersAndUrl(input) => headers_step(input),
        Step::EmbeddingRequestAndParams(input) => request_step(input),
        Step::ModelsUrlAndList(input) => models_step(input),
    }
}

fn base_policy() -> OpenAiConfig {
    OpenAiConfig {
        family: ConfigFamily::Other,
        api_key: None,
        headers: Headers::new(),
        settings: OpenAiSettings::default(),
    }
}

fn catalog(chat_models: &BTreeSet<String>, text_models: &BTreeSet<String>) -> CatalogCapabilities {
    CatalogCapabilities {
        chat_models: chat_models.clone(),
        text_models: text_models.clone(),
    }
}

fn environment_record(values: &BTreeMap<String, String>) -> VercelAiGatewayEnvironment {
    VercelAiGatewayEnvironment {
        vercel_ai_gateway_api_base: values.get("VERCEL_AI_GATEWAY_API_BASE").cloned(),
        vercel_ai_gateway_api_key: values
            .get("VERCEL_AI_GATEWAY_API_KEY")
            .map(String::as_str)
            .map(SecretString::from),
        vercel_oidc_token: values
            .get("VERCEL_OIDC_TOKEN")
            .map(String::as_str)
            .map(SecretString::from),
    }
}

fn provider_step(step: &ProviderStep) -> Value {
    let config = VercelAiGatewayConfig {
        base: base_policy(),
    };
    let key = step.api_key.as_deref().map(SecretString::from);
    let info = provider_info(
        step.api_base.as_deref(),
        key.as_ref(),
        &environment_record(&step.environment),
    );
    json!({
        "provider": VercelAiGatewayConfig::PROVIDER,
        "api_base": info.api_base,
        "api_key": info.api_key.as_ref().map(ExposeSecret::expose_secret),
        "supported_params": config.supported_params(&step.model, &catalog(&step.chat_models, &step.text_models)),
    })
}

fn map_step(step: &MapStep) -> Value {
    let config = VercelAiGatewayConfig {
        base: base_policy(),
    };
    let mut requested = step.non_default_params.clone();
    let mut mapped = step.optional_params.clone();
    config.map_openai_params(
        &step.model,
        &mut requested,
        &mut mapped,
        &catalog(&step.chat_models, &step.text_models),
    );
    json!({"optional_params": mapped, "remaining_params": requested})
}

fn headers_step(step: &HeadersStep) -> Value {
    let key = step.api_key.as_deref().map(SecretString::from);
    json!({
        "headers": validate_environment(&step.headers, key.as_ref()),
        "url": get_complete_url(step.api_base.as_deref(), &environment_record(&step.environment)),
    })
}

fn request_step(step: &RequestStep) -> Value {
    let mut mapped = step.optional_params.clone();
    map_embedding_params(&step.non_default_params, &mut mapped);
    json!({
        "request": transform_embedding_request(&step.model, &step.input, &mapped),
        "params": mapped,
        "supported": SUPPORTED_OPENAI_PARAMS,
    })
}

fn models_step(step: &ModelsStep) -> Value {
    let url = models_url(
        step.api_base.as_deref(),
        &environment_record(&step.environment),
    );
    let result = match parse_models(step.status, step.body.as_bytes()) {
        Ok(models) => json!({"models": models}),
        Err(error) => json!({"failure": error.to_string()}),
    };
    json!({"url": url, "result": result})
}

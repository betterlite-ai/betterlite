use std::collections::{BTreeMap, BTreeSet};

use betterlite_models::cost_usage::CostUsage;
use betterlite_models::token_cost::GenericCostResult;
use betterlite_models::token_cost::error::TokenCostError;
use betterlite_openai::ai21_chat::{
    SUPPORTED_OPENAI_PARAMS as AI21_PARAMS, map_openai_params as map_ai21_params,
};
use betterlite_openai::minimax_chat;
use betterlite_openai::modelscope_chat;
use betterlite_openai::openai_base_config::{
    CatalogCapabilities, ConfigFamily, Headers, Object, OpenAiConfig, OpenAiSettings,
};
use betterlite_openai::tencent_chat::{self, TencentParams};
use betterlite_openai::tencent_cost::cost_per_token;
use betterlite_openai::xai_usage::GenericTokenPricing;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;

use super::input::{
    Ai21MapStep, MinimaxInfoStep, MinimaxUrlStep, Step, TencentCostStep, TencentMapStep,
};

fn params_value(names: &[&str]) -> Value {
    Value::Array(
        names
            .iter()
            .map(|name| Value::String((*name).to_owned()))
            .collect(),
    )
}

fn info_value(base: String, key: Option<SecretString>) -> Value {
    let mut value = Object::new();
    value.insert(String::from("base"), Value::String(base));
    value.insert(
        String::from("key"),
        key.map_or(Value::Null, |secret| {
            Value::String(secret.expose_secret().to_owned())
        }),
    );
    Value::Object(value)
}

fn config() -> OpenAiConfig {
    OpenAiConfig {
        family: ConfigFamily::Other,
        api_key: None,
        headers: Headers::new(),
        settings: OpenAiSettings::default(),
    }
}

const fn catalog() -> CatalogCapabilities {
    CatalogCapabilities {
        chat_models: BTreeSet::new(),
        text_models: BTreeSet::new(),
    }
}

fn ai21_map(step: &Ai21MapStep) -> Value {
    let mut mapped = step.optional_params.clone();
    map_ai21_params(&step.params, &mut mapped);
    Value::Object(mapped)
}

fn minimax_info(step: &MinimaxInfoStep) -> Value {
    let api_key = step.api_key.as_deref().map(SecretString::from);
    let env_key = step.env_key.as_deref().map(SecretString::from);
    let global_key = step.global_key.as_deref().map(SecretString::from);
    let (base, key) = minimax_chat::provider_info(
        step.api_base.as_deref(),
        step.env_base.as_deref(),
        api_key.as_ref(),
        env_key.as_ref(),
        global_key.as_ref(),
    );
    info_value(base, key)
}

fn minimax_url(step: &MinimaxUrlStep) -> Value {
    Value::String(minimax_chat::complete_url(
        step.api_base.as_deref(),
        step.env_base.as_deref(),
    ))
}

fn tencent_map(step: &TencentMapStep) -> Value {
    let mut mapped = step.optional_params.clone();
    TencentParams {
        config: &config(),
        catalog: &catalog(),
        supports_reasoning: step.supports_reasoning,
        supports_adaptive_thinking: step.supports_adaptive_thinking,
    }
    .map(&step.model, &step.params, &mut mapped);
    Value::Object(mapped)
}

struct MissingPricing;

impl GenericTokenPricing for MissingPricing {
    fn generic_cost_per_token(
        &self,
        _model: &str,
        _custom_llm_provider: &str,
        _usage: &CostUsage,
    ) -> Result<GenericCostResult, TokenCostError> {
        Err(TokenCostError::UnknownModel)
    }
}

fn helper_error(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message,
    })
}

fn tencent_cost(step: &TencentCostStep) -> HelperResult {
    let total_tokens = step
        .prompt_tokens
        .checked_add(step.completion_tokens)
        .ok_or_else(|| helper_error("Overflow", String::from("the token counts overflow")))?;
    let usage = CostUsage {
        prompt_tokens: step.prompt_tokens,
        completion_tokens: step.completion_tokens,
        total_tokens,
        prompt_details: None,
        completion_details: None,
        cache_read_tokens: None,
        numeric_extras: BTreeMap::new(),
    };
    let (prompt, completion) = cost_per_token(&MissingPricing, &step.model, &usage)
        .map_err(|error| helper_error("UnknownModel", error.to_string()))?;
    let mut value = Object::new();
    value.insert(String::from("ok"), Value::Bool(true));
    value.insert(String::from("prompt"), Value::String(prompt.to_string()));
    value.insert(
        String::from("completion"),
        Value::String(completion.to_string()),
    );
    Ok(Value::Object(value))
}

pub(crate) fn execute(step: &Step) -> HelperResult {
    match step {
        Step::Ai21Params => Ok(params_value(&AI21_PARAMS)),
        Step::Ai21Map(given) => Ok(ai21_map(given)),
        Step::MinimaxParams(given) => Ok(params_value(&minimax_chat::supported_params(
            &given.model,
            &config(),
            &catalog(),
            given.supports_reasoning,
        ))),
        Step::MinimaxInfo(given) => Ok(minimax_info(given)),
        Step::MinimaxUrl(given) => Ok(minimax_url(given)),
        Step::ModelscopeUrl(given) => Ok(Value::String(modelscope_chat::complete_url(
            given.api_base.as_deref(),
        ))),
        Step::TencentParams(given) => Ok(params_value(&tencent_chat::supported_params(
            &given.model,
            &config(),
            &catalog(),
            given.supports_reasoning,
        ))),
        Step::TencentUrl(given) => Ok(Value::String(tencent_chat::complete_url(
            given.api_base.as_deref(),
        ))),
        Step::TencentMap(given) => Ok(tencent_map(given)),
        Step::TencentCost(given) => tencent_cost(given),
    }
}

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
use betterlite_openai::tencent_cost::{GenericTokenPricing, cost_per_token};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;

use crate::helpers::{HelperError, HelperResult};

use super::input::Step;

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
        key.map_or(Value::Null, |key| {
            Value::String(key.expose_secret().clone())
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

fn catalog() -> CatalogCapabilities {
    CatalogCapabilities {
        chat_models: BTreeSet::new(),
        text_models: BTreeSet::new(),
    }
}

fn ai21_map(params: &Object, optional_params: &Object) -> Value {
    let mut mapped = optional_params.clone();
    map_ai21_params(params, &mut mapped);
    Value::Object(mapped)
}

fn tencent_map(
    model: &str,
    supports_reasoning: bool,
    supports_adaptive_thinking: bool,
    params: &Object,
    optional_params: &Object,
) -> Value {
    let mut mapped = optional_params.clone();
    TencentParams {
        config: &config(),
        catalog: &catalog(),
        supports_reasoning,
        supports_adaptive_thinking,
    }
    .map(model, params, &mut mapped);
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

fn tencent_cost(model: &str, prompt_tokens: i64, completion_tokens: i64) -> HelperResult {
    let total_tokens = prompt_tokens
        .checked_add(completion_tokens)
        .ok_or_else(|| {
            Box::new(HelperError {
                class: String::from("Overflow"),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: String::from("the token counts overflow"),
            })
        })?;
    let usage = CostUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        prompt_details: None,
        completion_details: None,
        cache_read_tokens: None,
        numeric_extras: BTreeMap::new(),
    };
    let (prompt, completion) = cost_per_token(&MissingPricing, model, &usage).map_err(|error| {
        Box::new(HelperError {
            class: String::from("UnknownModel"),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })?;
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
        Step::Ai21Map {
            params,
            optional_params,
        } => Ok(ai21_map(params, optional_params)),
        Step::MinimaxParams {
            model,
            supports_reasoning,
        } => Ok(params_value(&minimax_chat::supported_params(
            model,
            &config(),
            &catalog(),
            *supports_reasoning,
        ))),
        Step::MinimaxInfo {
            api_base,
            env_base,
            api_key,
            env_key,
            global_key,
        } => {
            let api_key = api_key
                .as_ref()
                .map(|value| SecretString::from(value.as_str()));
            let env_key = env_key
                .as_ref()
                .map(|value| SecretString::from(value.as_str()));
            let global_key = global_key
                .as_ref()
                .map(|value| SecretString::from(value.as_str()));
            let (base, key) = minimax_chat::provider_info(
                api_base.as_deref(),
                env_base.as_deref(),
                api_key.as_ref(),
                env_key.as_ref(),
                global_key.as_ref(),
            );
            Ok(info_value(base, key))
        }
        Step::MinimaxUrl { api_base, env_base } => Ok(Value::String(minimax_chat::complete_url(
            api_base.as_deref(),
            env_base.as_deref(),
        ))),
        Step::ModelscopeUrl { api_base } => Ok(Value::String(modelscope_chat::complete_url(
            api_base.as_deref(),
        ))),
        Step::TencentParams {
            model,
            supports_reasoning,
        } => Ok(params_value(&tencent_chat::supported_params(
            model,
            &config(),
            &catalog(),
            *supports_reasoning,
        ))),
        Step::TencentUrl { api_base } => Ok(Value::String(tencent_chat::complete_url(
            api_base.as_deref(),
        ))),
        Step::TencentMap {
            model,
            supports_reasoning,
            supports_adaptive_thinking,
            params,
            optional_params,
        } => Ok(tencent_map(
            model,
            *supports_reasoning,
            *supports_adaptive_thinking,
            params,
            optional_params,
        )),
        Step::TencentCost {
            model,
            prompt_tokens,
            completion_tokens,
        } => tencent_cost(model, *prompt_tokens, *completion_tokens),
    }
}

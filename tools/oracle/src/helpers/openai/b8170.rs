use betterlite_models::cost_usage::{CompletionDetails, CostUsage, PromptDetails};
use betterlite_models::off_peak::schema::OffPeakBlockEntry;
use betterlite_models::price_lookup::PricedModel;
use betterlite_models::tiered_pricing::{Scalar, Tier};
use betterlite_models::token_cost::prompt_overlap::PromptExtras;
use betterlite_models::token_rates::ModelInfo;
use betterlite_openai::dashscope_chat::{ChatBasePolicy, DashscopeChatConfig};
use betterlite_openai::dashscope_cost::{CostRequest, cost_per_token};
use betterlite_openai::openai_base_config::CatalogCapabilities;
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Step {
    SupportedParams {
        model: String,
        #[serde(default)]
        chat_models: Vec<String>,
        #[serde(default)]
        text_models: Vec<String>,
    },
    ProviderInfo {
        policy: PolicyStep,
        #[serde(default)]
        api_base: Option<String>,
        #[serde(default)]
        api_key: Option<String>,
        #[serde(default)]
        env_api_key: Option<String>,
    },
    Cost {
        model_info: ModelInfoStep,
        usage: UsageStep,
        current_time: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyStep {
    default_base: String,
    #[serde(default)]
    env_base: Option<String>,
    #[serde(default)]
    env_in_complete_url: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelInfoStep {
    #[serde(default)]
    input_cost_per_token: Option<Scalar>,
    #[serde(default)]
    output_cost_per_token: Option<Scalar>,
    #[serde(default)]
    cache_read_input_token_cost: Option<Scalar>,
    #[serde(default, rename = "cache_creation_input_token_cost")]
    cache_creation: Option<Scalar>,
    #[serde(default, rename = "output_cost_per_reasoning_token")]
    reasoning: Option<Scalar>,
    #[serde(default)]
    tiered_pricing: Option<Vec<Tier>>,
    #[serde(default)]
    off_peak_pricing: Option<OffPeakBlockEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UsageStep {
    #[serde(rename = "prompt_tokens")]
    prompt: i64,
    #[serde(default, rename = "completion_tokens")]
    completion: Option<i64>,
    #[serde(default, rename = "cached_tokens")]
    cached: Option<i64>,
    #[serde(default, rename = "cache_creation_tokens")]
    cache_creation: Option<i64>,
    #[serde(default, rename = "reasoning_tokens")]
    reasoning: Option<i64>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        input
            .steps
            .iter()
            .map(execute)
            .collect::<Result<Vec<Value>, _>>()
            .map(Value::Array)
    }
}

fn execute(step: &Step) -> HelperResult {
    match step {
        Step::SupportedParams {
            model,
            chat_models,
            text_models,
        } => Ok(supported_params_value(model, chat_models, text_models)),
        Step::ProviderInfo {
            policy,
            api_base,
            api_key,
            env_api_key,
        } => Ok(provider_info_value(
            policy,
            api_base.as_deref(),
            api_key.as_deref(),
            env_api_key.as_deref(),
        )),
        Step::Cost {
            model_info,
            usage,
            current_time,
        } => cost_value(model_info, usage, current_time),
    }
}

fn supported_params_value(model: &str, chat_models: &[String], text_models: &[String]) -> Value {
    let catalog = CatalogCapabilities {
        chat_models: chat_models.iter().cloned().collect(),
        text_models: text_models.iter().cloned().collect(),
    };
    let params = DashscopeChatConfig::supported_params(model, &catalog);
    Value::Array(
        params
            .into_iter()
            .map(|name| Value::String(name.to_owned()))
            .collect(),
    )
}

fn provider_info_value(
    policy: &PolicyStep,
    api_base: Option<&str>,
    api_key: Option<&str>,
    env_api_key: Option<&str>,
) -> Value {
    let given_key = api_key.map(|value| SecretString::from(value.to_owned()));
    let info = config(policy, env_api_key).provider_info(api_base, given_key.as_ref());
    let mut value = Map::new();
    value.insert(String::from("api_base"), Value::String(info.api_base));
    value.insert(
        String::from("api_key"),
        info.api_key.map_or(Value::Null, |key| {
            Value::String(key.expose_secret().to_owned())
        }),
    );
    Value::Object(value)
}

fn config(policy: &PolicyStep, env_api_key: Option<&str>) -> DashscopeChatConfig {
    DashscopeChatConfig {
        policy: ChatBasePolicy {
            default_base: policy.default_base.clone(),
            env_base: policy.env_base.clone(),
            env_in_complete_url: policy.env_in_complete_url,
        },
        env_api_key: env_api_key.map(|value| SecretString::from(value.to_owned())),
    }
}

fn priced(step: &ModelInfoStep) -> PricedModel {
    let tiered_pricing = step.tiered_pricing.clone().unwrap_or_default();
    let mut record = ModelInfo::new();
    for (name, value) in [
        ("input_cost_per_token", &step.input_cost_per_token),
        ("output_cost_per_token", &step.output_cost_per_token),
        (
            "cache_read_input_token_cost",
            &step.cache_read_input_token_cost,
        ),
        ("cache_creation_input_token_cost", &step.cache_creation),
        ("output_cost_per_reasoning_token", &step.reasoning),
    ] {
        if let Some(scalar) = value {
            record.insert(name.to_owned(), scalar.clone());
        }
    }
    PricedModel {
        record,
        tiered_pricing,
        off_peak_pricing: step
            .off_peak_pricing
            .as_ref()
            .and_then(OffPeakBlockEntry::as_block)
            .cloned(),
        off_peak_cache_read_present: false,
    }
}

fn cost_usage(step: &UsageStep) -> CostUsage {
    CostUsage {
        prompt_tokens: step.prompt,
        completion_tokens: step.completion.unwrap_or(0),
        total_tokens: 0,
        prompt_details: Some(PromptDetails {
            text_tokens: None,
            audio_tokens: None,
            image_tokens: None,
            video_tokens: None,
            web_search_requests: None,
        }),
        completion_details: step.reasoning.map(|reasoning| CompletionDetails {
            text_tokens: None,
            audio_tokens: None,
            image_tokens: None,
            video_tokens: None,
            reasoning_tokens: Some(reasoning),
        }),
        cache_read_tokens: step.cached,
        numeric_extras: BTreeMap::new(),
    }
}

const fn prompt_extras(step: &UsageStep) -> PromptExtras {
    PromptExtras {
        cache_write_tokens: None,
        cache_creation_tokens: step.cache_creation,
        cache_creation_token_details: None,
        cached_tokens_details: None,
        character_count: None,
        image_count: None,
        video_length_seconds: None,
        audio_length_seconds: None,
        query_count: None,
    }
}

fn cost_value(model_info: &ModelInfoStep, usage: &UsageStep, current_time: &str) -> HelperResult {
    let moment = DateTime::parse_from_rfc3339(current_time)
        .map_err(|error| helper_error("InvalidInstant", error.to_string()))?
        .with_timezone(&Utc);
    let model = priced(model_info);
    let usage_record = cost_usage(usage);
    let extras = prompt_extras(usage);
    let request = CostRequest {
        model_info: &model,
        usage: &usage_record,
        prompt_extras: &extras,
        current_time: moment,
    };
    let result = cost_per_token(&request, &TimeZoneDatabase::from_env())
        .map_err(|error| helper_error("DashscopeCostError", error.to_string()))?;
    let mut value = Map::new();
    value.insert(
        String::from("prompt_cost"),
        Value::String(result.prompt_cost.normalize().to_string()),
    );
    value.insert(
        String::from("completion_cost"),
        Value::String(result.completion_cost.normalize().to_string()),
    );
    Ok(Value::Object(value))
}

fn helper_error(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

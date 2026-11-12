use betterlite_config::sections::mistral_options::Settings as EnvironmentSettings;
use betterlite_openai::base_helpers::provider_config;
use betterlite_openai::mistral_options::{
    MistralSettings, REASONING_SYSTEM_PROMPT, ReasoningEffortSelector, map_openai_params,
    provider_info, supported_openai_params,
};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

mod tests;

const STRENGTH_ORDER: [&str; 6] = ["minimal", "low", "medium", "high", "xhigh", "max"];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    settings: MistralSettings,
    #[serde(default)]
    catalog: Catalog,
    steps: Vec<Step>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Catalog {
    #[serde(default)]
    supports_reasoning: bool,
    #[serde(default)]
    declared_efforts: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Step {
    Options {
        model: String,
        max_depth: i64,
        #[serde(default)]
        params: Vec<(String, Value)>,
        #[serde(default)]
        optional_params: Map<String, Value>,
    },
    ProviderInfo {
        #[serde(default)]
        api_base: Option<String>,
        #[serde(default)]
        api_key: Option<String>,
        #[serde(default)]
        azure_api_base: Option<String>,
        #[serde(default)]
        azure_api_key: Option<String>,
        #[serde(default)]
        mistral_api_key: Option<String>,
    },
    Prompt,
}

struct CatalogSelector<'catalog> {
    declared: Option<&'catalog [String]>,
}

impl ReasoningEffortSelector for CatalogSelector<'_> {
    fn accepted_effort(&self, _model: &str, requested: &str) -> Option<String> {
        let declared = self.declared?;
        let ranked: Vec<&str> = STRENGTH_ORDER
            .iter()
            .copied()
            .filter(|effort| declared.iter().any(|item| item == effort))
            .collect();
        let Some(requested_rank) = STRENGTH_ORDER.iter().position(|known| *known == requested)
        else {
            return Some(requested.to_owned());
        };
        if ranked.is_empty() || ranked.contains(&requested) {
            return Some(requested.to_owned());
        }
        let nearest = ranked.iter().find(|effort| {
            STRENGTH_ORDER
                .iter()
                .position(|known| *known == **effort)
                .is_some_and(|rank| rank >= requested_rank)
        });
        let accepted = nearest.copied().or_else(|| ranked.last().copied())?;
        Some(accepted.to_owned())
    }
}

fn invalid(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("InvalidInput"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn options_value(
    input: &Input,
    model: &str,
    max_depth: i64,
    params: &[(String, Value)],
    optional_params: &Map<String, Value>,
) -> Result<Value, Box<HelperError>> {
    let selector = CatalogSelector {
        declared: input.catalog.declared_efforts.as_deref(),
    };
    let mut mapped = optional_params.clone();
    map_openai_params(params, &mut mapped, model, &selector, max_depth);
    let config = provider_config(&input.settings).map_err(|error| invalid(error.to_string()))?;
    Ok(json!({
        "model": model,
        "config": config,
        "supported": supported_openai_params(model, input.catalog.supports_reasoning),
        "mapped": mapped,
    }))
}

fn provider_value(
    api_base: Option<&str>,
    api_key: Option<&str>,
    azure_api_base: Option<&str>,
    azure_api_key: Option<&str>,
    mistral_api_key: Option<&str>,
) -> Value {
    let environment = EnvironmentSettings {
        azure_api_base: azure_api_base.map(str::to_owned),
        azure_api_key: azure_api_key.map(SecretString::from),
        api_key: mistral_api_key.map(SecretString::from),
        max_recurse_depth: 100,
    };
    let (base, key) = provider_info(api_base, api_key, &environment);
    let rendered_key = key.as_ref().map(|secret| secret.expose_secret().to_owned());
    json!({ "api_base": base, "api_key": rendered_key })
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        input
            .steps
            .iter()
            .map(|step| match step {
                Step::Options {
                    model,
                    max_depth,
                    params,
                    optional_params,
                } => options_value(input, model, *max_depth, params, optional_params),
                Step::ProviderInfo {
                    api_base,
                    api_key,
                    azure_api_base,
                    azure_api_key,
                    mistral_api_key,
                } => Ok(provider_value(
                    api_base.as_deref(),
                    api_key.as_deref(),
                    azure_api_base.as_deref(),
                    azure_api_key.as_deref(),
                    mistral_api_key.as_deref(),
                )),
                Step::Prompt => Ok(json!({ "prompt": REASONING_SYSTEM_PROMPT })),
            })
            .collect::<Result<Vec<Value>, _>>()
            .map(Value::Array)
    }
}

use std::collections::BTreeMap;

use fancy_regex::Regex;
use url::Url;

use crate::arguments::json::Json;
use crate::cache_accounting::constants::{
    GPT_VERSION_PATTERN, MIN_BREAKPOINT_GPT_VERSION, OPENAI_API_BASE_ENV_VARS, OPENAI_API_HOST,
};

// The two lookups the dialect decision borrows: the model cost map and provider resolution, both injected.
pub trait ModelFacts {
    fn cache_breakpoint_flag(&self, model: &str) -> Option<bool>;

    // None is where LiteLLM's get_llm_provider raises BadRequestError.
    fn provider(&self, model: &str) -> Option<String>;
}

// Everything outside the chosen dialect that the prompt hook reads; the environment is a snapshot, never the process.
pub struct DialectInputs<'input> {
    pub model: &'input str,
    pub custom_llm_provider: Option<&'input str>,
    pub api_base: Option<&'input str>,
    pub base_url: Option<&'input str>,
    pub global_api_base: Option<&'input str>,
    pub environment: &'input BTreeMap<String, String>,
    pub prompt_cache_options: Option<&'input Json>,
}

#[must_use]
pub fn model_map_breakpoint_flag(model: &str, facts: &dyn ModelFacts) -> Option<bool> {
    let suffix = model.rsplit('/').next().unwrap_or(model);
    let flags = [
        facts.cache_breakpoint_flag(model),
        facts.cache_breakpoint_flag(suffix),
    ];
    flags.into_iter().flatten().next()
}

#[must_use]
pub fn gpt_version(model: &str) -> Option<(u64, u64)> {
    let segment = model.rsplit('/').next().unwrap_or(model).to_lowercase();
    let pattern = Regex::new(GPT_VERSION_PATTERN).ok()?;
    let captured = pattern.captures(&segment).ok()??;
    let major = captured.get(1)?.as_str().parse::<u64>().ok()?;
    let minor = match captured.get(2) {
        Some(minor) => minor.as_str().parse::<u64>().ok()?,
        None => 0,
    };
    Some((major, minor))
}

#[must_use]
pub fn supports_openai_breakpoint(model: &str, facts: &dyn ModelFacts) -> bool {
    if let Some(flag) = model_map_breakpoint_flag(model, facts) {
        return flag;
    }
    gpt_version(model).is_some_and(|version| version >= MIN_BREAKPOINT_GPT_VERSION)
}

// LiteLLM's chain skips an empty candidate, so an empty api_base falls through to the next source.
#[must_use]
pub fn targets_openai_api(inputs: &DialectInputs<'_>) -> bool {
    let Some(base) = first_base(inputs) else {
        return true;
    };
    let Ok(parsed) = Url::parse(base) else {
        return false;
    };
    let suffix = format!(".{OPENAI_API_HOST}");
    parsed.host_str().is_some_and(|host| {
        let lowered = host.to_lowercase();
        lowered == OPENAI_API_HOST || lowered.ends_with(&suffix)
    })
}

fn first_base<'inputs>(inputs: &DialectInputs<'inputs>) -> Option<&'inputs str> {
    let environment = OPENAI_API_BASE_ENV_VARS
        .iter()
        .filter_map(|name| inputs.environment.get(*name).map(String::as_str));
    [inputs.api_base, inputs.base_url, inputs.global_api_base]
        .into_iter()
        .flatten()
        .chain(environment)
        .find(|value| !value.is_empty())
}

#[must_use]
pub fn targets_openai_breakpoint(inputs: &DialectInputs<'_>, facts: &dyn ModelFacts) -> bool {
    if !supports_openai_breakpoint(inputs.model, facts) {
        return false;
    }
    let declared = inputs.custom_llm_provider.filter(|name| !name.is_empty());
    let provider = declared
        .map(str::to_owned)
        .or_else(|| facts.provider(inputs.model));
    if provider.as_deref() != Some("openai") {
        return false;
    }
    inputs
        .prompt_cache_options
        .is_some_and(|options| !matches!(options, Json::Null))
        || targets_openai_api(inputs)
}

use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};
use std::sync::Mutex;

use betterlite_core::cost::context::ProviderInference;
use betterlite_core::environment_validation::error::EnvironmentValidationError;
use betterlite_core::environment_validation::probe::{
    ModelProbe, ProbeCompletion, check_valid_key, valid_model,
};
use betterlite_core::environment_validation::{EnvironmentRequest, validate_environment};
use betterlite_core::retry_after::{
    RetryAttempt, RetryTiming, calculate_retry_after, retry_after_from_header, should_retry,
};
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::provider_model_sets::ProviderModelSets;
use betterlite_models::resolution::{
    ProviderIdentity, ResolutionContext, ResolutionError, ResolutionRequest, resolve,
};
use jiff::Timestamp;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{HelperError, HelperResult};

pub(crate) struct Op;

// The sets are keyed by LiteLLM's attribute names; `vertex_ai` stands for `litellm.models_by_provider["vertex_ai"]`.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct ModelSetsInput {
    #[serde(default)]
    vertex_ai: BTreeSet<String>,
    #[serde(flatten)]
    sets: BTreeMap<String, BTreeSet<String>>,
}

// Each operation answers a list of cases in order, so one file can sweep every provider row.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, tag = "operation", rename_all = "snake_case")]
pub(crate) enum Input {
    ValidateEnvironment { cases: Vec<EnvironmentInput> },
    ValidModel { cases: Vec<ValidModelInput> },
    CheckValidKey { cases: Vec<KeyInput> },
    ShouldRetry { cases: Vec<StatusInput> },
    RetryAfterHeader { cases: Vec<HeaderInput> },
    CalculateRetryAfter { cases: Vec<BackoffInput> },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EnvironmentInput {
    #[serde(default)]
    environment: BTreeSet<String>,
    #[serde(default)]
    model_sets: ModelSetsInput,
    model: Option<String>,
    api_key: Option<String>,
    api_base: Option<String>,
    api_version: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ValidModelInput {
    #[serde(default)]
    model_sets: ModelSetsInput,
    model: String,
    fails: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct KeyInput {
    model: String,
    api_key: String,
    fails: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StatusInput {
    status_code: u16,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HeaderInput {
    headers: Option<BTreeMap<String, String>>,
    now: i64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BackoffInput {
    remaining_retries: i32,
    max_retries: i32,
    #[serde(default)]
    min_timeout: i32,
    headers: Option<BTreeMap<String, String>>,
    now: i64,
    random: f64,
    initial_retry_delay: Option<f64>,
    max_retry_delay: Option<f64>,
    jitter: Option<f64>,
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

// get_llm_provider with no catalog entries, so only a provider prefix names a provider.
struct Resolver {
    catalog: ModelCatalog,
    names: BTreeSet<String>,
    environment: BTreeMap<String, String>,
}

impl ProviderInference for Resolver {
    type Failure = ResolutionError;

    fn inferred_provider(&self, model: &str) -> Result<ProviderIdentity, ResolutionError> {
        let context = ResolutionContext {
            catalog: &self.catalog,
            known_model_names: &self.names,
            custom_providers: &self.names,
            environment: &self.environment,
            registry: JSONProviderRegistry::load(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        };
        let request = ResolutionRequest {
            model,
            custom_llm_provider: None,
            api_base: None,
            api_key: None,
        };
        resolve(&request, &context).map(|resolved| resolved.provider)
    }
}

fn model_sets(input: &ModelSetsInput) -> Result<ProviderModelSets, Box<HelperError>> {
    let mut built = ProviderModelSets::default();
    for (name, models) in &input.sets {
        let Some((_, members)) = built
            .sets
            .iter_mut()
            .find(|(set, _)| set.to_string() == *name)
        else {
            return Err(invalid(format!("{name} is not a model set")));
        };
        members.clone_from(models);
    }
    built
        .models_by_provider
        .insert(String::from("vertex_ai"), input.vertex_ai.clone());
    Ok(built)
}

// A probe that records what it was asked and either works or refuses.
struct RecordingProbe {
    fails: bool,
    calls: Mutex<Vec<Value>>,
}

impl RecordingProbe {
    const fn new(fails: bool) -> Self {
        Self {
            fails,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn answer(&self, call: Value) -> Result<(), &'static str> {
        let Ok(mut calls) = self.calls.lock() else {
            return Err("the call log is poisoned");
        };
        calls.push(call);
        if self.fails {
            Err("provider refused")
        } else {
            Ok(())
        }
    }

    fn recorded(&self) -> Vec<Value> {
        self.calls.lock().map_or_else(
            |poisoned| poisoned.into_inner().clone(),
            |calls| calls.clone(),
        )
    }
}

impl ModelProbe for RecordingProbe {
    type Failure = &'static str;

    fn retrieve_openai_model(
        &self,
        model: &str,
    ) -> impl Future<Output = Result<(), &'static str>> + Send {
        ready(self.answer(json!({"kind": "retrieve", "model": model})))
    }

    fn complete(
        &self,
        request: &ProbeCompletion<'_>,
    ) -> impl Future<Output = Result<(), &'static str>> + Send {
        ready(self.answer(json!({
            "kind": "completion",
            "model": request.model,
            "user_message": request.user_message,
            "api_key": request.api_key.map(|key| key.expose_secret().to_owned()),
            "max_tokens": request.max_tokens,
        })))
    }
}

fn header_value(headers: Option<&BTreeMap<String, String>>) -> Option<&str> {
    headers
        .and_then(|all| all.get("retry-after"))
        .map(String::as_str)
}

fn runtime() -> Result<tokio::runtime::Runtime, Box<HelperError>> {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(|source| invalid(source.to_string()))
}

fn environment_answer(input: &EnvironmentInput) -> HelperResult {
    let resolver = Resolver {
        catalog: ModelCatalog::parse(b"{}", UnknownFields::Warn)
            .map_err(|source| invalid(source.to_string()))?,
        names: BTreeSet::new(),
        environment: BTreeMap::new(),
    };
    let key = input.api_key.as_deref().map(SecretString::from);
    let request = EnvironmentRequest {
        model: input.model.as_deref(),
        api_key: key.as_ref(),
        api_base: input.api_base.as_deref(),
        api_version: input.api_version.as_deref(),
    };
    let sets = model_sets(&input.model_sets)?;
    let check = validate_environment(&request, &input.environment, &sets, &resolver);
    Ok(json!({
        "keys_in_environment": check.keys_in_environment,
        "missing_keys": check.missing_keys,
    }))
}

fn valid_model_answer(input: &ValidModelInput) -> HelperResult {
    let probe = RecordingProbe::new(input.fails);
    let sets = model_sets(&input.model_sets)?;
    let outcome = runtime()?.block_on(valid_model(&probe, &sets, &input.model));
    let error = match outcome {
        Ok(()) => Value::Null,
        Err(EnvironmentValidationError::BadRequest { model: refused }) => {
            json!({"status_code": 400_u16, "llm_provider": "", "model": refused})
        }
        Err(other) => return Err(invalid(other.to_string())),
    };
    Ok(json!({"calls": probe.recorded(), "error": error}))
}

fn key_answer(input: &KeyInput) -> HelperResult {
    let probe = RecordingProbe::new(input.fails);
    let key = SecretString::from(input.api_key.as_str());
    let valid = runtime()?.block_on(check_valid_key(&probe, &input.model, &key));
    Ok(json!({"calls": probe.recorded(), "valid": valid}))
}

fn header_answer(input: &HeaderInput) -> HelperResult {
    let now = Timestamp::from_second(input.now).map_err(|source| invalid(source.to_string()))?;
    let seconds = retry_after_from_header(header_value(input.headers.as_ref()), now);
    Ok(json!({"seconds": seconds}))
}

fn backoff_answer(input: &BackoffInput) -> HelperResult {
    let defaults = RetryTiming::default();
    let timing = RetryTiming {
        initial_retry_delay: input
            .initial_retry_delay
            .unwrap_or(defaults.initial_retry_delay),
        max_retry_delay: input.max_retry_delay.unwrap_or(defaults.max_retry_delay),
        jitter: input.jitter.unwrap_or(defaults.jitter),
    };
    let attempt = RetryAttempt {
        remaining_retries: input.remaining_retries,
        max_retries: input.max_retries,
        min_timeout: input.min_timeout,
    };
    let now = Timestamp::from_second(input.now).map_err(|source| invalid(source.to_string()))?;
    let wait = calculate_retry_after(
        &timing,
        &attempt,
        header_value(input.headers.as_ref()),
        now,
        input.random,
    );
    Ok(json!({"wait": wait}))
}

fn answers<Case>(cases: &[Case], answer: fn(&Case) -> HelperResult) -> HelperResult {
    cases
        .iter()
        .map(answer)
        .collect::<Result<Vec<_>, _>>()
        .map(Value::Array)
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ValidateEnvironment { cases } => answers(cases, environment_answer),
            Input::ValidModel { cases } => answers(cases, valid_model_answer),
            Input::CheckValidKey { cases } => answers(cases, key_answer),
            Input::ShouldRetry { cases } => answers(cases, |status| {
                Ok(json!({"retry": should_retry(status.status_code)}))
            }),
            Input::RetryAfterHeader { cases } => answers(cases, header_answer),
            Input::CalculateRetryAfter { cases } => answers(cases, backoff_answer),
        }
    }
}

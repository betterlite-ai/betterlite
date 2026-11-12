use betterlite_azure::azure_default_chain::environment::ChainEnvironment;
use betterlite_azure::azure_default_chain::error::DefaultChainError;
use betterlite_azure::azure_default_chain::provider::{
    AzureTokenProvider, ProviderInner, ProviderRequest, get_azure_ad_token_provider,
    try_default_credential,
};
use betterlite_config::sections::azure_default_chain::AzureCredentialType;
use serde_json::{Value, json};

use super::fixture::{Recorder, capture};
use super::{ChainStep, InferStep, ProviderStep, TryStep, environment_of, requested};

fn selection_log(logs: &[String]) -> Option<String> {
    logs.iter()
        .find(|line| line.starts_with("For Azure AD Token Provider"))
        .cloned()
}

fn success_log(logs: &[String]) -> Option<String> {
    logs.iter()
        .find(|line| line.contains(" acquired a token from "))
        .cloned()
}

fn warning_log(logs: &[String]) -> Option<String> {
    logs.iter()
        .find(|line| line.contains("failed to retrieve a token"))
        .cloned()
}

fn error_value(error: &DefaultChainError) -> Value {
    let (variable, missing) = error.detail();
    let message = match error {
        DefaultChainError::MissingEnvironment { .. }
        | DefaultChainError::UnsupportedFilter { .. }
        | DefaultChainError::MissingArguments { .. } => None,
        DefaultChainError::InvalidFilter { .. }
        | DefaultChainError::Construction { .. }
        | DefaultChainError::Credential { .. }
        | DefaultChainError::Exhausted { .. } => Some(error.to_string()),
    };
    json!({
        "category": error.category(),
        "variable": variable,
        "message": message,
        "missing": missing.map(<[&'static str]>::to_vec).unwrap_or_default(),
    })
}

pub(super) fn infer_steps(steps: &[InferStep]) -> Value {
    let results: Vec<Value> = steps
        .iter()
        .map(|step| match environment_of(&step.environment) {
            Ok(environment) => json!({
                "credential": environment.infer_credential_type().as_str()
            }),
            Err(error) => json!({ "error": error.message }),
        })
        .collect();
    json!({ "steps": results })
}

fn run_provider<'env, 'grants>(
    environment: &'env ChainEnvironment,
    recorder: &'grants Recorder,
    scope: Option<&'env str>,
    credential: Option<AzureCredentialType>,
) -> (
    Result<AzureTokenProvider<'grants, Recorder>, DefaultChainError>,
    Vec<String>,
) {
    capture(|| {
        get_azure_ad_token_provider(&ProviderRequest {
            scope,
            credential,
            environment,
            grants: recorder,
        })
    })
}

fn provider_report(
    provider: &AzureTokenProvider<'_, Recorder>,
    recorder: &Recorder,
    log: Option<&str>,
) -> Value {
    json!({
        "kind": "credential",
        "chosen": provider.credential.as_str(),
        "scope": provider.scope,
        "constructed": recorder.records(),
        "warnings": provider.warnings,
        "log": log,
    })
}

pub(super) fn provider_value(
    step: &ProviderStep,
) -> Result<Value, Box<crate::helpers::HelperError>> {
    let environment = environment_of(&step.environment)?;
    let recorder = Recorder::new(&[], &step.construct_errors);
    let credential = requested(step.credential.as_deref())?;
    let (result, logs) = run_provider(&environment, &recorder, step.scope.as_deref(), credential);
    let log = selection_log(&logs);
    Ok(match result {
        Ok(provider) => provider_report(&provider, &recorder, log.as_deref()),
        Err(error) => json!({
            "kind": "error",
            "error": error_value(&error),
            "constructed": recorder.records(),
            "log": log,
        }),
    })
}

pub(super) fn chain_value(step: &ChainStep) -> Result<Value, Box<crate::helpers::HelperError>> {
    let environment = environment_of(&step.environment)?;
    let recorder = Recorder::new(&step.scripts, &step.construct_errors);
    let credential = requested(step.credential.as_deref())?;
    let outcome = run_provider(&environment, &recorder, step.scope.as_deref(), credential);
    Ok(chain_outcome(&recorder, outcome))
}

fn chain_outcome(
    recorder: &Recorder,
    outcome: (
        Result<AzureTokenProvider<'_, Recorder>, DefaultChainError>,
        Vec<String>,
    ),
) -> Value {
    let (result, logs) = outcome;
    match result {
        Ok(provider) => chain_result(&provider, recorder, &logs, selection_log(&logs).as_deref()),
        Err(error) => json!({
            "kind": "error",
            "error": error_value(&error),
            "constructed": recorder.records(),
            "log": selection_log(&logs),
        }),
    }
}

fn chain_result(
    provider: &AzureTokenProvider<'_, Recorder>,
    recorder: &Recorder,
    logs: &[String],
    log: Option<&str>,
) -> Value {
    let chain = match &provider.inner {
        ProviderInner::Chain(chain) => chain.name.as_str(),
        ProviderInner::Single(_) => provider.credential.as_str(),
    };
    let mut report = json!({
        "kind": "token",
        "chosen": provider.credential.as_str(),
        "scope": provider.scope,
        "chain": chain,
        "constructed": recorder.records(),
        "warnings": provider.warnings,
        "log": log,
    });
    let (result, execution_logs) = capture(|| provider.token());
    if let Some(map) = report.as_object_mut() {
        map.insert("token_calls".to_owned(), json!(recorder.calls()));
    }
    let all_logs: Vec<String> = logs.iter().chain(&execution_logs).cloned().collect();
    match result {
        Ok(token) => {
            if let Some(map) = report.as_object_mut() {
                map.insert(
                    "token".to_owned(),
                    json!(secrecy::ExposeSecret::expose_secret(&token.token)),
                );
                map.insert("acquired".to_owned(), json!(success_log(&all_logs)));
            }
        }
        Err(error) => {
            if let Some(map) = report.as_object_mut() {
                map.insert("kind".to_owned(), json!("chain_error"));
                map.insert("message".to_owned(), json!(error.to_string()));
                map.insert("warning".to_owned(), json!(warning_log(&all_logs)));
            }
        }
    }
    report
}

fn boundary_log(line: &str) -> bool {
    line.starts_with("Attempting to use DefaultAzureCredential")
        || line.starts_with("Successfully obtained Azure AD token provider")
        || line.starts_with("DefaultAzureCredential failed: ")
}

fn invocation_result(error: &DefaultChainError) -> Value {
    match error {
        DefaultChainError::Exhausted { message, .. } => {
            json!({ "kind": "chain_error", "message": message })
        }
        DefaultChainError::MissingEnvironment { .. }
        | DefaultChainError::InvalidFilter { .. }
        | DefaultChainError::UnsupportedFilter { .. }
        | DefaultChainError::Construction { .. }
        | DefaultChainError::Credential { .. }
        | DefaultChainError::MissingArguments { .. } => json!({
            "kind": "error",
            "category": error.category(),
            "message": error.to_string(),
        }),
    }
}

pub(super) fn boundary_value(step: &TryStep) -> Result<Value, Box<crate::helpers::HelperError>> {
    let environment = environment_of(&step.environment)?;
    let recorder = Recorder::new(&step.scripts, &step.construct_errors);
    let (boundary, logs) = capture(|| try_default_credential(&step.scope, &environment, &recorder));
    let failure = boundary
        .failure
        .as_ref()
        .map(|error| format!("DefaultAzureCredential failed: {error}"));
    let filtered: Vec<String> = logs.into_iter().filter(|line| boundary_log(line)).collect();
    let mut report = json!({
        "kind": "boundary",
        "provider": boundary.provider.is_some(),
        "constructed": recorder.records(),
        "logs": filtered,
        "failure": failure,
        "warnings": [],
        "result": Value::Null,
    });
    if let Some(provider) = boundary.provider {
        if let Some(map) = report.as_object_mut() {
            map.insert("warnings".to_owned(), json!(provider.warnings));
        }
        if step.invoke {
            let (result, _invoke_logs) = capture(|| provider.token());
            if let Some(map) = report.as_object_mut() {
                map.insert(
                    "result".to_owned(),
                    match result {
                        Ok(token) => json!({
                            "kind": "token",
                            "token": secrecy::ExposeSecret::expose_secret(&token.token)
                        }),
                        Err(error) => invocation_result(&error),
                    },
                );
            }
        }
    }
    Ok(report)
}

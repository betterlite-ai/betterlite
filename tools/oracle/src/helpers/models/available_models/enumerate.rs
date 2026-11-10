use std::mem::take;
use std::sync::{Mutex, PoisonError};

use betterlite_models::available_models::cache::AvailableModelsCache;
use betterlite_models::available_models::environment::EnvironmentSnapshot;
use betterlite_models::available_models::{
    EnumerationRequest, EnumerationWarning, get_valid_models,
};
use secrecy::SecretString;
use serde_json::{Value, json};

use super::clock;
use super::input::{CallInput, EnumerateInput};
use super::listing::ScriptedListing;

pub(super) fn run(fields: &EnumerateInput) -> Value {
    let environment = EnvironmentSnapshot::from(&fields.environment);
    let (state, switchable) = clock::switchable();
    let mut cache = AvailableModelsCache::with_clock(switchable);
    let mut calls = Vec::new();
    for call in &fields.calls {
        state.lock().unwrap_or_else(PoisonError::into_inner).fails = call.cache_clock_fails;
        calls.push(run_call(call, fields, &environment, &mut cache));
    }
    json!({ "calls": calls })
}

fn run_call(
    call: &CallInput,
    fields: &EnumerateInput,
    environment: &EnvironmentSnapshot,
    cache: &mut AvailableModelsCache,
) -> Value {
    let listing = ScriptedListing {
        outcomes: call.listing.clone(),
        records: Mutex::new(Vec::new()),
    };
    let request = EnumerationRequest {
        check_provider_endpoint: call.check_provider_endpoint,
        global_check_provider_endpoint: call
            .global_check_provider_endpoint
            .unwrap_or(fields.global_check_provider_endpoint),
        custom_llm_provider: call.custom_llm_provider.as_deref(),
        parameters: call.litellm_params.clone(),
        api_key: call.api_key.clone().map(SecretString::from),
        api_base: call.api_base.clone(),
        provider_list: &fields.provider_list,
        models_by_provider: &fields.models_by_provider,
        environment,
    };
    let outcome = get_valid_models(&request, cache, &listing);
    let warnings: Vec<Value> = outcome
        .warnings
        .iter()
        .map(|warning| json!({"kind": warning_kind(*warning)}))
        .collect();
    let records = take(
        &mut *listing
            .records
            .lock()
            .unwrap_or_else(PoisonError::into_inner),
    );
    json!({"models": outcome.models, "warnings": warnings, "listings": records})
}

const fn warning_kind(warning: EnumerationWarning) -> &'static str {
    match warning {
        EnumerationWarning::ErrorGettingValidModels => "error_getting_valid_models",
    }
}

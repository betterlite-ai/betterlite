use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::sync::Arc;

use betterlite_core::api::realtime_connect::dispatch::arealtime;
use betterlite_core::api::realtime_connect::{
    RealtimeConnectCall, RealtimeConnectError, RealtimeConnection,
};
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_oracle_helper_api::error::HelperError;
use indexmap::IndexMap;
use serde_json::{Value, json};

use super::super::Scenario;
use super::fixture::{Fixture, ScriptedTokens, TestConfig, logging_state, scripted_tokens};
use super::observation::observation;

pub(crate) async fn run_scenario(scenario: &Scenario) -> Result<Value, Box<HelperError>> {
    let tokens = scripted_tokens(scenario.token.as_ref());
    let token_source = Arc::clone(&tokens);
    let fixture = Fixture {
        tokens: token_source,
    };
    let (catalog, names, environment) = (ModelCatalog::default(), BTreeSet::new(), BTreeMap::new());
    let resolution = ResolutionContext {
        catalog: &catalog,
        known_model_names: &names,
        custom_providers: &names,
        environment: &environment,
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let mut logging = logging_state()?;
    let litellm_params = IndexMap::new();
    let call = RealtimeConnectCall {
        model: scenario.model.as_str(),
        params: &scenario.params,
        query_params: scenario.query_params.as_ref(),
        client: None,
        client_sent_beta: false,
        litellm_params: &litellm_params,
    };
    let result = arealtime(
        &fixture,
        &scenario.settings,
        &mut logging,
        &resolution,
        &call,
    )
    .await;
    observed(result, &tokens)
}

fn observed(
    result: Result<RealtimeConnection<TestConfig>, RealtimeConnectError<io::Error>>,
    tokens: &ScriptedTokens,
) -> Result<Value, Box<HelperError>> {
    match result {
        Ok(connection) => Ok(observation(connection, tokens)),
        Err(RealtimeConnectError::UnsupportedModel { model }) => Ok(json!({
            "branch": "error",
            "model": null,
            "error": format!("Unsupported model: {model}"),
        })),
        Err(error) => Err(Box::new(HelperError {
            class: String::from("RuntimeError"),
            status: None,
            error_type: Some(None),
            param: Some(None),
            code: Some(None),
            message: error.to_string(),
        })),
    }
}

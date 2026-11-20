use std::collections::BTreeMap;

use betterlite_config::sections::foundry_credentials::Settings;
use betterlite_foundry::credentials::model_info::ModelInfoAccessors;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_provider::foundry_resource_dispatch::responses_decisions::{
    Call, Dispatch, Outcome,
};
use betterlite_provider::foundry_resource_dispatch::{
    FoundryResourceOperation, FoundryResourceResult, dispatch,
};
use serde::Deserialize;
use serde_json::json;
use tokio::runtime::Runtime;

use super::support::runtime;

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    operation: Operation,
    api_base: Option<String>,
    global_api_base: Option<String>,
    api_key: Option<String>,
    #[serde(default)]
    model: String,
    #[serde(default)]
    environment: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    Url,
    DecisionsRoute,
    NoNativeWebsocket,
}

pub(super) fn answer(input: &Input) -> HelperResult {
    let runtime = runtime()?;
    // the case's environment is the AZURE_AI_* layer, as the Responses leaf reads it in production
    let settings = Settings::resolve_environment(&input.environment);
    let found = match input.operation {
        Operation::Url => outcome(
            Call::Url {
                accessors: accessors(input, &settings),
                api_base: input.api_base.as_deref(),
            },
            &runtime,
        ),
        Operation::DecisionsRoute => outcome(
            Call::DecisionsUrl {
                api_base: input.api_base.as_deref().unwrap_or_default(),
                model: &input.model,
            },
            &runtime,
        ),
        Operation::NoNativeWebsocket => outcome(Call::Policy, &runtime),
    };
    Ok(match found {
        Some(Outcome::Url(url)) => json!({"kind": "url", "url": url}),
        Some(Outcome::DecisionsUrl(url)) => json!({
            "kind": "decisions_route",
            "url": url,
            "api_base": &input.api_base,
            "api_key": &input.api_key,
        }),
        Some(Outcome::Policy {
            provider,
            native_websocket,
        }) => json!({
            "kind": "no_native_websocket",
            "provider": provider.to_string(),
            "websocket": native_websocket,
        }),
        None => json!({"kind": "error"}),
    })
}

fn accessors<'env>(input: &'env Input, settings: &'env Settings) -> ModelInfoAccessors<'env> {
    ModelInfoAccessors {
        global_api_base: input.global_api_base.as_deref(),
        global_api_key: None,
        environment_api_base: settings.api_base.as_deref(),
        environment_api_key: None,
    }
}

fn outcome(call: Call<'_>, runtime: &Runtime) -> Option<Outcome> {
    let operation = FoundryResourceOperation::ResponsesDecisions(Box::new(Dispatch { call }));
    match runtime.block_on(dispatch(operation)) {
        Ok(FoundryResourceResult::ResponsesDecisions(resolved)) => Some(resolved),
        Ok(_) | Err(_) => None,
    }
}

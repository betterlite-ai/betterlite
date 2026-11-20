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
use serde_json::{Value, json};
use tokio::runtime::Runtime;

use super::support::runtime;

#[derive(Debug, Deserialize)]
pub(super) struct Input {
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

pub(super) fn run(input: &Input) -> HelperResult {
    let runtime = runtime()?;
    Ok(match input.operation {
        Operation::Url => url_answer(input, &runtime),
        Operation::DecisionsRoute => decisions_answer(input, &runtime),
        Operation::NoNativeWebsocket => policy_answer(&runtime),
    })
}

fn url_answer(input: &Input, runtime: &Runtime) -> Value {
    // the case's environment is the AZURE_AI_* layer, as the Responses leaf reads it in production
    let settings = Settings::resolve_environment(&input.environment);
    let accessors = ModelInfoAccessors {
        global_api_base: input.global_api_base.as_deref(),
        global_api_key: None,
        environment_api_base: settings.api_base.as_deref(),
        environment_api_key: None,
    };
    let operation = FoundryResourceOperation::ResponsesDecisions(Box::new(Dispatch {
        call: Call::Url {
            accessors,
            api_base: input.api_base.as_deref(),
        },
    }));
    match runtime.block_on(dispatch(operation)) {
        Ok(FoundryResourceResult::ResponsesDecisions(Outcome::Url(url))) => {
            json!({"kind": "url", "url": url})
        }
        Ok(_) | Err(_) => json!({"kind": "error"}),
    }
}

fn decisions_answer(input: &Input, runtime: &Runtime) -> Value {
    let operation = FoundryResourceOperation::ResponsesDecisions(Box::new(Dispatch {
        call: Call::DecisionsUrl {
            api_base: input.api_base.as_deref().unwrap_or_default(),
            model: &input.model,
        },
    }));
    match runtime.block_on(dispatch(operation)) {
        Ok(FoundryResourceResult::ResponsesDecisions(Outcome::DecisionsUrl(url))) => json!({
            "kind": "decisions_route",
            "url": url,
            "api_base": &input.api_base,
            "api_key": &input.api_key,
        }),
        Ok(_) | Err(_) => json!({"kind": "error"}),
    }
}

fn policy_answer(runtime: &Runtime) -> Value {
    let operation =
        FoundryResourceOperation::ResponsesDecisions(Box::new(Dispatch { call: Call::Policy }));
    match runtime.block_on(dispatch(operation)) {
        Ok(FoundryResourceResult::ResponsesDecisions(Outcome::Policy {
            provider,
            native_websocket,
        })) => json!({
            "kind": "no_native_websocket",
            "provider": provider.to_string(),
            "websocket": native_websocket,
        }),
        Ok(_) | Err(_) => json!({"kind": "error"}),
    }
}

mod answers;

use betterlite_core::api::messages_reasoning_route::AutoSummary;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::json;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

use self::answers::{normalize_answer, plain_answer, route_answer};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Call {
    NormalizeEffort {
        kwargs: IndexMap<String, Json>,
        model_info: Option<IndexMap<String, Json>>,
    },
    Route {
        kwargs: IndexMap<String, Json>,
        thinking: Json,
        model_info: Option<IndexMap<String, Json>>,
        provider: Option<ResolvedScript>,
        global_value: Option<bool>,
        environment: Option<String>,
    },
    PlainEffort {
        kwargs: IndexMap<String, Json>,
        thinking: Json,
        provider: Option<ResolvedScript>,
        bridge: BridgeScript,
    },
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResolvedScript {
    local: String,
    provider: String,
    api_base: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BridgeScript {
    mode: Option<String>,
    fails: bool,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut calls = Vec::with_capacity(input.calls.len());
        for call in &input.calls {
            calls.push(answer(call)?);
        }
        Ok(json!({ "calls": calls }))
    }
}

fn answer(call: &Call) -> HelperResult {
    match call {
        Call::NormalizeEffort { kwargs, model_info } => {
            normalize_answer(kwargs, model_info.as_ref())
        }
        Call::Route {
            kwargs,
            thinking,
            model_info,
            provider,
            global_value,
            environment,
        } => route_answer(
            kwargs,
            thinking,
            model_info.as_ref(),
            provider.as_ref(),
            AutoSummary {
                global: global_value.unwrap_or(false),
                environment: environment.as_deref(),
            },
        ),
        Call::PlainEffort {
            kwargs,
            thinking,
            provider,
            bridge,
        } => plain_answer(kwargs, thinking, provider.as_ref(), bridge),
    }
}

fn helper_error(class: &str, status: Option<u16>, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message,
    })
}

mod answers;

use async_trait::async_trait;
use betterlite_core::api::responses_hook_policy::callbacks::{
    CallbackLists, CallbackValue, DeploymentHookDispatcher, NamedCallbackResolver, PolicyCallback,
    RESPONSES_CALL_TYPE, ResponsesHookOutcome, ResponsesHookRequest, run_responses_pre_hook,
};
use betterlite_core::api::responses_hook_policy::safety::ApiSurface;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::HelperError;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};

use answers::{followup_answer, safety_answer, settings_answer};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use std::convert::Infallible;
use tokio::runtime::Builder;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    DeploymentHook {
        model: String,
        input: Json,
        custom_llm_provider: String,
        optional_params: IndexMap<String, Json>,
        generic_params: IndexMap<String, Json>,
        global_callbacks: Vec<CallbackSpec>,
        #[serde(default)]
        dynamic_callbacks: Vec<CallbackSpec>,
    },
    Settings {
        probes: Vec<IndexMap<String, Json>>,
    },
    Safety {
        tool_calls: Json,
        fingerprints: Vec<Json>,
        depth: i64,
        max_loops: u32,
        model: String,
        blocks: Vec<Json>,
        stream: bool,
        api_surface: ApiSurface,
    },
    Followup {
        request_kwargs: IndexMap<String, Json>,
        patch_kwargs: IndexMap<String, Json>,
        request_params: Vec<String>,
        depth: i64,
        max_loops: u32,
        fingerprints: Vec<Json>,
        fingerprint: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum CallbackSpec {
    Override {
        #[serde(default)]
        result: Option<IndexMap<String, Json>>,
    },
    Plain,
    UnresolvedString,
    Other,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::DeploymentHook { .. } => deployment_hook(input),
            Input::Settings { .. } => Ok(settings_answer(input)),
            Input::Safety { .. } => Ok(safety_answer(input)),
            Input::Followup { .. } => followup_answer(input),
        }
    }
}

fn deployment_hook(input: &Input) -> HelperResult {
    let Input::DeploymentHook {
        model,
        input: hook_input,
        custom_llm_provider,
        optional_params,
        generic_params,
        global_callbacks,
        dynamic_callbacks,
    } = input
    else {
        return Err(failure(
            "InvalidInput",
            "expected a deployment_hook input".to_owned(),
        ));
    };
    let global: Vec<CallbackValue> = global_callbacks.iter().map(callback_value).collect();
    let dynamic: Vec<CallbackValue> = dynamic_callbacks.iter().map(callback_value).collect();
    let lists = CallbackLists {
        global: &global,
        dynamic: &dynamic,
    };
    let resolver = NamedCallbacks;
    let replacements = global_callbacks.iter().map(scripted_result).collect();
    let dispatcher = ScriptedDispatcher { replacements };
    let request = hook_request(
        model,
        hook_input,
        custom_llm_provider,
        optional_params,
        generic_params,
    );
    let outcome = block_on_hook(&lists, &resolver, &dispatcher, request)?;
    Ok(deployment_json(&outcome))
}

fn block_on_hook(
    lists: &CallbackLists<'_>,
    resolver: &NamedCallbacks,
    dispatcher: &ScriptedDispatcher,
    request: ResponsesHookRequest,
) -> Result<ResponsesHookOutcome, Box<HelperError>> {
    let runtime = Builder::new_current_thread()
        .build()
        .map_err(|error| failure("RuntimeError", error.to_string()))?;
    runtime
        .block_on(run_responses_pre_hook(request, lists, resolver, dispatcher))
        .map_err(|error| failure("DeploymentHookError", error.to_string()))
}

fn hook_request(
    model: &str,
    input: &Json,
    custom_llm_provider: &str,
    optional_params: &IndexMap<String, Json>,
    generic_params: &IndexMap<String, Json>,
) -> ResponsesHookRequest {
    ResponsesHookRequest {
        model: model.to_owned(),
        input: input.clone(),
        custom_llm_provider: custom_llm_provider.to_owned(),
        optional_params: optional_params.clone(),
        generic_params: generic_params.clone(),
    }
}

fn policy(name: &str, overrides: bool) -> PolicyCallback {
    PolicyCallback {
        name: name.to_owned(),
        overrides_pre_hook: overrides,
        agentic: None,
    }
}

struct NamedCallbacks;

impl NamedCallbackResolver for NamedCallbacks {
    // a fresh LiteLLM process holds no logger instance that a name could resolve to
    fn resolve(&self, _name: &str) -> Option<PolicyCallback> {
        None
    }
}

fn callback_value(spec: &CallbackSpec) -> CallbackValue {
    match spec {
        CallbackSpec::Override { .. } => CallbackValue::Object(policy("override", true)),
        CallbackSpec::Plain => CallbackValue::Object(policy("plain", false)),
        CallbackSpec::UnresolvedString => CallbackValue::Named("langfuse".to_owned()),
        CallbackSpec::Other => CallbackValue::Other,
    }
}

// stands in for B157's dispatcher: global objects run in order, each non-null result updating the request
struct ScriptedDispatcher {
    replacements: Vec<Option<IndexMap<String, Json>>>,
}

fn scripted_result(spec: &CallbackSpec) -> Option<IndexMap<String, Json>> {
    match spec {
        CallbackSpec::Override { result } => result.clone(),
        CallbackSpec::Plain | CallbackSpec::UnresolvedString | CallbackSpec::Other => None,
    }
}

#[async_trait]
impl DeploymentHookDispatcher for ScriptedDispatcher {
    type Error = Infallible;

    async fn pre_call_deployment_hook(
        &self,
        request: IndexMap<String, Json>,
        call_type: &str,
    ) -> Result<Option<IndexMap<String, Json>>, Self::Error> {
        let mut modified = request;
        // the scripted callback answers only the responses call type, as the source's callback does
        if call_type == RESPONSES_CALL_TYPE {
            for result in self.replacements.iter().flatten() {
                modified.extend(result.clone());
            }
        }
        Ok(Some(modified))
    }
}

fn deployment_json(outcome: &ResponsesHookOutcome) -> Value {
    json!({
        "model": outcome.model,
        "input": outcome.input,
        "custom_llm_provider": outcome.custom_llm_provider,
        "optional_params": outcome.optional_params,
        "generic_params": {
            "api_key": outcome.generic_params.get("api_key").unwrap_or(&Json::Null),
            "api_base": outcome.generic_params.get("api_base").unwrap_or(&Json::Null),
        },
    })
}

pub(super) fn failure(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

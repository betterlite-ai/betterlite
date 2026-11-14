use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::prompt_dispatch::factory::prompt_factory;
use betterlite_router::prompt_dispatch::known_callbacks::KNOWN_CALLBACKS;
use betterlite_router::prompt_dispatch::ports::{CompilePromptCall, CompiledPrompt, PromptPorts};
use betterlite_router::prompt_dispatch::predicate::is_prompt_model;
use serde::Deserialize;
use serde_json::{Value, json};
use thiserror::Error;
use tokio::sync::Mutex;

use crate::helpers::{Helper, HelperError, HelperResult};

const LOGGING_HANDLE: &str = "logging-handle";
const ORIGINAL_FUNCTION: &str = "acompletion";
const TRACE_ID: &str = "trace-id";
// Router.acompletion fills these in itself and the Python side drops them from every answer: the trace id is random and the original function is a bound method.
const ROUTER_ONLY_KEYS: [&str; 2] = ["litellm_trace_id", "original_function"];

#[derive(Debug, Deserialize)]
#[serde(tag = "check", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    IsPromptModel {
        model: String,
        #[serde(default)]
        model_list: Vec<DeploymentRow>,
    },
    PromptFactory {
        model: String,
        #[serde(default)]
        model_list: Vec<DeploymentRow>,
        request: JsonObject,
        compiled: CompiledRow,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeploymentRow {
    model_name: String,
    #[serde(default)]
    litellm_params: JsonObject,
    #[serde(default)]
    model_info: JsonObject,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CompiledRow {
    model: String,
    #[serde(default)]
    messages: Vec<Json>,
    #[serde(default)]
    optional_params: JsonObject,
}

#[derive(Debug, Error)]
#[error("no deployment is named {0}")]
struct NoDeployment(String);

struct Outcome {
    call: &'static str,
    request: JsonObject,
}

struct HelperPorts {
    deployments: Vec<DeploymentSpec>,
    compiled: CompiledPrompt,
    compile_call: Mutex<Value>,
}

#[async_trait]
impl PromptPorts for HelperPorts {
    type Error = NoDeployment;
    type Response = Outcome;

    async fn select_deployment(
        &self,
        model: &str,
        _messages: &[Json],
        _specific_deployment: Option<&Json>,
        _request: &JsonObject,
    ) -> Result<DeploymentSpec, NoDeployment> {
        self.deployments
            .iter()
            .find(|deployment| deployment.model_name == model)
            .cloned()
            .ok_or_else(|| NoDeployment(model.to_owned()))
    }

    fn update_with_deployment(&self, _request: &mut JsonObject, _deployment: &DeploymentSpec) {}

    async fn compile_prompt(
        &self,
        call: CompilePromptCall<'_>,
    ) -> Result<CompiledPrompt, NoDeployment> {
        let mut request_kwargs = call.request.clone();
        let mut non_default_params = call.non_default_params.clone();
        request_kwargs.shift_remove("litellm_logging_obj");
        for key in ROUTER_ONLY_KEYS {
            request_kwargs.shift_remove(key);
            non_default_params.shift_remove(key);
        }
        *self.compile_call.lock().await = json!({
            "model": call.model,
            "messages": call.messages,
            "non_default_params": non_default_params,
            "prompt_id": call.prompt_id,
            "prompt_variables": call.prompt_variables,
            "prompt_label": call.prompt_label,
            "request_kwargs": request_kwargs,
            "injected_for_every_deployment": call.injected_for_every_deployment,
        });
        Ok(self.compiled.clone())
    }

    fn has_deployments(&self, model: &str) -> bool {
        self.deployments
            .iter()
            .any(|deployment| deployment.model_name == model)
    }

    async fn call_direct(&self, request: JsonObject) -> Result<Outcome, NoDeployment> {
        Ok(Outcome {
            call: "direct",
            request,
        })
    }

    async fn call_with_fallbacks(&self, request: JsonObject) -> Result<Outcome, NoDeployment> {
        Ok(Outcome {
            call: "with_fallbacks",
            request,
        })
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::IsPromptModel { model, model_list } => {
                let named: Vec<DeploymentSpec> = specs(model_list)
                    .into_iter()
                    .filter(|deployment| deployment.model_name == *model)
                    .collect();
                Ok(json!({"is_prompt_model": is_prompt_model(&named, &KNOWN_CALLBACKS)}))
            }
            Input::PromptFactory {
                model,
                model_list,
                request,
                compiled,
            } => {
                let ports = HelperPorts {
                    deployments: specs(model_list),
                    compiled: CompiledPrompt {
                        model: compiled.model.clone(),
                        messages: compiled.messages.clone(),
                        optional_params: compiled.optional_params.clone(),
                    },
                    compile_call: Mutex::new(Value::Null),
                };
                let mut request = router_request(model, request);
                let handle = Json::Text(LOGGING_HANDLE.to_owned());
                request.insert("litellm_logging_obj".to_owned(), handle.clone());
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|error| value_error(error.to_string()))?;
                let mut outcome = runtime
                    .block_on(prompt_factory(model, &[], request, &ports))
                    .map_err(|error| value_error(error.to_string()))?;
                let logging_kept =
                    outcome.request.shift_remove("litellm_logging_obj") == Some(handle);
                let original_function_kept = outcome.request.contains_key("original_function");
                for key in ROUTER_ONLY_KEYS {
                    outcome.request.shift_remove(key);
                }
                Ok(json!({
                    "compile_call": ports.compile_call.into_inner(),
                    "request": outcome.request,
                    "logging_kept": logging_kept,
                    "original_function_kept": original_function_kept,
                    "call": outcome.call,
                }))
            }
        }
    }
}

// What Router.acompletion puts in the kwargs before it asks whether the model is a prompt management one.
fn router_request(model: &str, request: &JsonObject) -> JsonObject {
    let mut kwargs = request.clone();
    kwargs.insert("model".to_owned(), Json::Text(model.to_owned()));
    kwargs.insert("messages".to_owned(), Json::List(Vec::new()));
    kwargs.insert("stream".to_owned(), Json::Bool(false));
    kwargs.insert(
        "original_function".to_owned(),
        Json::Text(ORIGINAL_FUNCTION.to_owned()),
    );
    kwargs
        .entry("litellm_trace_id".to_owned())
        .or_insert_with(|| Json::Text(TRACE_ID.to_owned()));
    let metadata = kwargs
        .entry("metadata".to_owned())
        .or_insert_with(|| Json::Object(JsonObject::new()));
    if let Json::Object(fields) = metadata {
        fields.insert("model_group".to_owned(), Json::Text(model.to_owned()));
        fields.insert("model_group_alias".to_owned(), Json::Null);
    }
    kwargs
}

fn specs(rows: &[DeploymentRow]) -> Vec<DeploymentSpec> {
    rows.iter()
        .map(|row| DeploymentSpec {
            model_name: row.model_name.clone(),
            litellm_params: row.litellm_params.clone(),
            model_info: row.model_info.clone(),
            extra: JsonObject::new(),
        })
        .collect()
}

fn value_error(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

mod editor;
mod logs;

use std::collections::BTreeMap;

use async_trait::async_trait;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_core::api::messages_compaction::dispatch::{
    DispatchRequest, apply_context_management,
};
use betterlite_core::api::messages_compaction::error::DispatchError;
use betterlite_core::api::messages_context_clear::result::PolyfillResult;
use betterlite_core::api::summary_execution::completion::{
    SummaryCall, SummaryCallFailure, SummaryCompletion,
};
use betterlite_core::api::summary_execution::extract::CompletionAnswer;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_prompts::arguments::json::Object;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

use self::editor::{ANTHROPIC_ERROR, Script, ScriptFailure, ScriptedEditor};
use self::logs::{Diagnostic, install};

// the dispatcher's whole input: every scenario is one apply_context_management call
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    scenarios: Vec<Scenario>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Scenario {
    #[serde(default = "default_model")]
    model: String,
    messages: Vec<Json>,
    #[serde(default)]
    system: Option<Json>,
    #[serde(default)]
    tools: Option<Vec<Json>>,
    #[serde(default)]
    context_management: Option<Json>,
    #[serde(default)]
    litellm_metadata: Option<Object>,
    #[serde(default)]
    router: bool,
    #[serde(default)]
    principal: bool,
    #[serde(default)]
    clear: Vec<Script>,
    #[serde(default)]
    compact: Vec<Script>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .or_value_error()?;
        let mut runs = Vec::with_capacity(input.scenarios.len());
        for scenario in &input.scenarios {
            runs.push(run_scenario(&runtime, scenario)?);
        }
        Ok(json!({ "runs": runs }))
    }
}

fn run_scenario(runtime: &Runtime, scenario: &Scenario) -> Result<Value, Box<HelperError>> {
    let editor = ScriptedEditor::new(scenario.clear.clone(), scenario.compact.clone());
    let router = NeverCalled;
    let router_ref: Option<&dyn SummaryCompletion> = scenario.router.then_some(&router);
    let principal = scenario.principal.then(build_principal).transpose()?;
    let request = DispatchRequest {
        model: &scenario.model,
        messages: scenario.messages.clone(),
        tools: scenario.tools.clone(),
        system: scenario.system.clone(),
        context_management_spec: scenario.context_management.as_ref(),
        litellm_metadata: scenario.litellm_metadata.as_ref(),
        llm_router: router_ref,
        user_api_key_auth: principal.as_ref(),
    };
    let (logs, guard) = install();
    let outcome = runtime.block_on(apply_context_management(request, &editor));
    drop(guard);
    assemble(outcome, &editor, &logs.diagnostics())
}

fn assemble(
    outcome: Result<PolyfillResult, DispatchError<ScriptFailure>>,
    editor: &ScriptedEditor,
    diagnostics: &[Diagnostic],
) -> Result<Value, Box<HelperError>> {
    let (result, failure) = match outcome {
        Ok(polyfilled) => (shape(&polyfilled), Value::Null),
        Err(DispatchError::Editor(editor_failure)) if editor_failure.class == ANTHROPIC_ERROR => (
            Value::Null,
            failure_value(
                &editor_failure.class,
                editor_failure.status_code,
                &editor_failure.message,
            ),
        ),
        // any other exception escapes the whole run, as dispatch.py reports it
        Err(DispatchError::Editor(editor_failure)) => {
            return Err(helper_error(
                editor_failure.class,
                editor_failure.status_code,
                editor_failure.message,
            ));
        }
        // LiteLLM's mapping error is an ordinary exception too, never the 400 class
        Err(DispatchError::Spec(error)) => {
            return Err(helper_error(
                String::from("ContextManagementError"),
                None,
                error.to_string(),
            ));
        }
    };
    Ok(json!({
        "result": result,
        "failure": failure,
        "calls": editor.calls(),
        "diagnostics": diagnostics,
    }))
}

// the PolyfillResult as dict(polyfilled) shows it: every field present, the absent ones null
fn shape(result: &PolyfillResult) -> Value {
    json!({
        "messages": result.messages,
        "system": result.system,
        "applied_edits": result.applied_edits,
        "compaction_block": result.compaction_block,
        "iterations_usage": result.iterations_usage,
    })
}

fn failure_value(class: &str, status_code: Option<u16>, message: &str) -> Value {
    json!({
        "class": class,
        "status_code": status_code,
        "message": message,
    })
}

// an error that is not the client's 400 escapes the helper, as Python lets any other exception through
fn helper_error(class: String, status: Option<u16>, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class,
        status,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message,
    })
}

fn build_principal() -> Result<UserAPIKeyAuth, Box<HelperError>> {
    let fields: BTreeMap<String, JsonData> =
        serde_json::from_str(r#"{"user_id":"helper"}"#).or_value_error()?;
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).or_value_error()
}

fn default_model() -> String {
    String::from("main-model")
}

// a router the scripted editor only sees as present; it is never called
struct NeverCalled;

#[async_trait]
impl SummaryCompletion for NeverCalled {
    async fn complete(
        &self,
        _call: SummaryCall<'_>,
    ) -> Result<CompletionAnswer, SummaryCallFailure> {
        Err(SummaryCallFailure {
            message: String::from("the dispatcher never calls the router"),
        })
    }
}

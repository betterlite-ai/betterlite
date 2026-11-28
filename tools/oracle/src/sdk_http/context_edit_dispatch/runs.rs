use std::io;
use std::sync::Arc;

use betterlite_config::sections::summary_model_access::Settings;
use betterlite_core::api::messages_compaction::dispatch::{
    DispatchRequest, apply_context_management,
};
use betterlite_core::api::messages_compaction::editor::{EditorFailure, RegisteredEditors};
use betterlite_core::api::messages_compaction::error::DispatchError;
use betterlite_core::api::messages_context_clear::result::PolyfillResult;
use betterlite_core::api::summary_execution::completion::ExecutorCompletion;
use betterlite_core::api::summary_execution::error::SummaryExecutionError;

use super::super::error::SdkHttpError;
use super::super::result::kinds::SdkResult;
use super::support::{FixedCounter, NoBudgets, NoChecks, NoHooks, NoRows, deployment, executor};
use super::{Call, Failure, Outcome, Run, Scenario, Shape};

// the registered editors with the policy seams present in name only, as the case runs without a principal
type Editors<'services> =
    RegisteredEditors<'services, NoChecks, NoRows, NoBudgets, NoHooks, FixedCounter>;

// the client error the dispatcher lets through: B2432's trigger validation, as compact.py raises it
const ANTHROPIC_ERROR: &str = "AnthropicContextManagementError";

pub(super) async fn execute(call: &Call, api_base: &str) -> Result<SdkResult, SdkHttpError> {
    let mut runs = Vec::with_capacity(call.scenarios.len());
    for scenario in &call.scenarios {
        runs.push(run(scenario, api_base).await?);
    }
    Ok(SdkResult::ContextEditDispatch {
        value: Box::new(Outcome { runs }),
    })
}

async fn run(scenario: &Scenario, api_base: &str) -> Result<Run, SdkHttpError> {
    let chat = executor()?;
    let target = deployment(api_base)?;
    let direct = ExecutorCompletion {
        executor: &chat,
        deployment: &target,
    };
    let settings = Settings {
        // ast-grep-ignore: no-long-names
        context_management_summary_model: scenario.summary_model.clone(),
        // ast-grep-ignore: no-long-names
        context_management_summary_max_tokens: scenario.max_tokens,
    };
    let counter = FixedCounter {
        count: scenario.counter.count,
    };
    let editors: Editors<'_> = RegisteredEditors {
        settings: Some(&settings),
        proxy: None,
        budgets: None,
        hooks: None,
        history_counter: &counter,
        clear_counter: Arc::new(FixedCounter {
            count: scenario.counter.count,
        }),
        direct: &direct,
    };
    let request = DispatchRequest {
        model: &scenario.model,
        messages: scenario.messages.clone(),
        tools: scenario.tools.clone(),
        system: scenario.system.clone(),
        context_management_spec: scenario.context_management.as_ref(),
        litellm_metadata: None,
        llm_router: None,
        user_api_key_auth: None,
    };
    assemble(apply_context_management(request, &editors).await)
}

fn assemble(
    outcome: Result<PolyfillResult, DispatchError<EditorFailure<io::Error>>>,
) -> Result<Run, SdkHttpError> {
    let (result, failure) = match outcome {
        Ok(value) => (Some(shape(value)), None),
        Err(DispatchError::Editor(EditorFailure::Compact(SummaryExecutionError::Trigger(
            trigger,
        )))) => (
            None,
            Some(Failure {
                class: String::from(ANTHROPIC_ERROR),
                status_code: Some(trigger.status_code),
                message: trigger.message,
            }),
        ),
        // any other editor failure would abort the Python child, so it aborts the native run too
        Err(error) => {
            return Err(SdkHttpError::Invalid(format!(
                "the dispatcher failed outside a client error: {error}"
            )));
        }
    };
    Ok(Run { result, failure })
}

fn shape(result: PolyfillResult) -> Shape {
    Shape {
        messages: result.messages,
        system: result.system,
        applied_edits: result.applied_edits,
        compaction_block: result.compaction_block,
        iterations_usage: result.iterations_usage,
    }
}

use std::mem::take;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_config::sections::summary_model_access::Settings;
use betterlite_core::api::messages_chat_prepare::prepare::{
    ContextRequest, DispatchFailure, PolyfillDispatch, PrepareError,
};
// ast-grep-ignore: no-long-names
use betterlite_core::api::messages_chat_prepare::prepare::prepare_context_managed_request;
use betterlite_core::api::messages_compaction::dispatch::DispatchRequest;
use betterlite_core::api::messages_compaction::editor::RegisteredEditors;
use betterlite_core::api::messages_context_clear::result::PolyfillResult;
use betterlite_core::api::summary_execution::completion::{ExecutorCompletion, SummaryCompletion};

use super::super::error::SdkHttpError;
use super::super::result::kinds::SdkResult;
use super::support::{
    FixedCounter, NoBudgets, NoChecks, NoHooks, NoRows, deployment, executor, principal,
};
use super::{Call, CallRecord, Dispatcher, Failure, Outcome, Run, Scenario, Shape};

// the registered editors with the policy seams present in name only, as the case runs without a proxy
type Editors<'services> =
    RegisteredEditors<'services, NoChecks, NoRows, NoBudgets, NoHooks, FixedCounter>;

// the client error the wrapper lets through unchanged, as handler.py re-raises it
const ANTHROPIC_ERROR: &str = "AnthropicContextManagementError";

pub(super) async fn execute(call: &Call, api_base: &str) -> Result<SdkResult, SdkHttpError> {
    let mut runs = Vec::with_capacity(call.scenarios.len());
    for scenario in &call.scenarios {
        runs.push(run(scenario, api_base).await?);
    }
    Ok(SdkResult::MessagesContextPreparation {
        value: Box::new(Outcome { runs }),
    })
}

async fn run(scenario: &Scenario, api_base: &str) -> Result<Run, SdkHttpError> {
    let chat = executor()?;
    let target = deployment(api_base)?;
    let completion = ExecutorCompletion {
        executor: &chat,
        deployment: &target,
    };
    let counter = FixedCounter {
        count: scenario.counter,
    };
    let settings = summary_settings(scenario);
    let editors: Editors<'_> = RegisteredEditors {
        settings: Some(&settings),
        proxy: None,
        budgets: None,
        hooks: None,
        history_counter: &counter,
        clear_counter: Arc::new(FixedCounter {
            count: scenario.counter,
        }),
        direct: &completion,
    };
    let router: &dyn SummaryCompletion = &completion;
    let principal = scenario.principal.as_ref().map(principal).transpose()?;
    let request = context_request(
        scenario,
        principal.as_ref(),
        scenario.router.then_some(router),
    );
    let scripted = scripted_result(scenario);
    let recorder = Recorder::default();
    let recorded = Recorded {
        calls: recorder.clone(),
        answer: answer(scenario.dispatcher, &editors, scripted.as_ref())?,
    };
    // ast-grep-ignore: no-long-names
    let outcome = prepare_context_managed_request(request, &recorded).await;
    assemble(
        outcome,
        take(&mut *recorder.0.lock().unwrap_or_else(PoisonError::into_inner)),
    )
}

fn summary_settings(scenario: &Scenario) -> Settings {
    Settings {
        // ast-grep-ignore: no-long-names
        context_management_summary_model: scenario.summary_model.clone(),
        // ast-grep-ignore: no-long-names
        context_management_summary_max_tokens: scenario.max_tokens,
    }
}

fn context_request<'request>(
    scenario: &'request Scenario,
    principal: Option<&'request UserAPIKeyAuth>,
    router: Option<&'request dyn SummaryCompletion>,
) -> ContextRequest<'request> {
    ContextRequest {
        model: &scenario.model,
        messages: &scenario.messages,
        tools: scenario.tools.as_deref(),
        system: scenario.system.as_ref(),
        context_management_spec: scenario.context_management.as_ref(),
        additional_drop_params: scenario.additional_drop_params.as_ref(),
        litellm_metadata: scenario.metadata.as_ref(),
        llm_router: router,
        user_api_key_auth: principal,
    }
}

fn scripted_result(scenario: &Scenario) -> Option<PolyfillResult> {
    scenario.scripted.as_ref().map(|scripted| PolyfillResult {
        messages: scripted.messages.clone(),
        system: scripted.system.clone(),
        applied_edits: Vec::new(),
        compaction_block: None,
        iterations_usage: None,
    })
}

fn answer<'dispatcher, 'scripted>(
    mode: Dispatcher,
    editors: &'dispatcher dyn PolyfillDispatch,
    scripted: Option<&'scripted PolyfillResult>,
) -> Result<Answer<'dispatcher, 'scripted>, SdkHttpError> {
    match mode {
        Dispatcher::Real => Ok(Answer::Real(editors)),
        Dispatcher::Result => {
            let Some(result) = scripted else {
                return Err(SdkHttpError::Invalid(String::from(
                    "the scripted dispatcher needs a scripted result",
                )));
            };
            Ok(Answer::Result(result))
        }
        Dispatcher::Null => Ok(Answer::Null),
        Dispatcher::Error => Ok(Answer::Error),
    }
}

fn assemble(
    outcome: Result<Option<PolyfillResult>, PrepareError>,
    calls: Vec<CallRecord>,
) -> Result<Run, SdkHttpError> {
    let (result, failure) = match outcome {
        Ok(value) => (
            value.map(|result| Shape {
                messages: result.messages,
                system: result.system,
                applied_edits: result.applied_edits,
                compaction_block: result.compaction_block,
                iterations_usage: result.iterations_usage,
            }),
            None,
        ),
        Err(PrepareError::Context(error)) => (
            None,
            Some(Failure {
                class: String::from(ANTHROPIC_ERROR),
                status_code: error.status_code,
                message: error.message,
            }),
        ),
        // any other wrapper failure would abort the Python child, so it aborts the native run too
        Err(error) => {
            return Err(SdkHttpError::Invalid(format!(
                "the wrapper failed outside a client error: {error}"
            )));
        }
    };
    Ok(Run {
        result,
        failure,
        calls,
    })
}

enum Answer<'dispatcher, 'scripted> {
    Real(&'dispatcher dyn PolyfillDispatch),
    Result(&'scripted PolyfillResult),
    Null,
    Error,
}

// the wrapper's dispatcher as the case scripts it: the registered editors, a result, nothing, or a broken editor
struct Recorded<'dispatcher, 'scripted> {
    calls: Recorder,
    answer: Answer<'dispatcher, 'scripted>,
}

#[async_trait]
impl PolyfillDispatch for Recorded<'_, '_> {
    async fn dispatch(
        &self,
        request: DispatchRequest<'_>,
    ) -> Result<Option<PolyfillResult>, DispatchFailure> {
        // what the wrapper handed the dispatcher, recorded before the real editors or the scripted answer run
        self.calls.push(CallRecord {
            model: request.model.to_owned(),
            messages: request.messages.clone(),
            tools: request.tools.clone(),
            system: request.system.clone(),
            spec: request.context_management_spec.cloned(),
            metadata: request.litellm_metadata.cloned(),
            has_router: request.llm_router.is_some(),
            has_principal: request.user_api_key_auth.is_some(),
        });
        match &self.answer {
            Answer::Real(editors) => PolyfillDispatch::dispatch(*editors, request).await,
            Answer::Result(result) => Ok(Some((*result).clone())),
            Answer::Null => Ok(None),
            Answer::Error => Err(DispatchFailure::Other(String::from("broken"))),
        }
    }
}

#[derive(Clone, Default)]
struct Recorder(Arc<Mutex<Vec<CallRecord>>>);

impl Recorder {
    fn push(&self, record: CallRecord) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(record);
    }
}

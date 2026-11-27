use std::collections::BTreeMap;
use std::io;

use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_config::sections::summary_model_access::Settings;
use betterlite_core::api::messages_context_clear::result::PolyfillResult;
use betterlite_core::api::summary_execution::completion::SummaryCompletion;
use betterlite_core::api::summary_execution::editor::{
    EditorRequest, EditorServices, apply_compact_20260112,
};
use betterlite_core::api::summary_execution::error::SummaryExecutionError;
use betterlite_core::api::summary_model_access::access::ProxyGlobals;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_prompts::arguments::json::Object;

use super::super::error::SdkHttpError;
use super::super::result::kinds::SdkResult;
use super::logs::capture;
use super::ports::{Port, deployment, executor};
use super::support::{
    NoRows, Recorder, ScriptedBudgets, ScriptedChecks, ScriptedCounter, ScriptedHooks,
    ScriptedLimiter,
};
use super::{Call, CallRecord, Diagnostic, Failure, Outcome, Route, Run, Scenario, Shape};

pub(super) async fn execute(call: &Call, api_base: &str) -> Result<SdkResult, SdkHttpError> {
    let mut runs = Vec::with_capacity(call.scenarios.len());
    for scenario in &call.scenarios {
        runs.push(run(scenario, api_base).await?);
    }
    Ok(SdkResult::SummaryExecution {
        value: Box::new(Outcome { runs }),
    })
}

async fn run(scenario: &Scenario, api_base: &str) -> Result<Run, SdkHttpError> {
    let recorder = Recorder::default();
    let parts = Parts::new(scenario, recorder.clone());
    let principal = scenario.principal.as_ref().map(principal).transpose()?;
    let executor = executor()?;
    let deployment = deployment(api_base)?;
    let routed = Port::new(Route::Router, &executor, &deployment);
    let direct = Port::new(Route::Direct, &executor, &deployment);
    let (logs, guard) = capture();
    let outcome = apply(scenario, &parts, &routed, &direct, principal.as_ref()).await;
    drop(guard);
    let mut calls = routed.records();
    calls.extend(direct.records());
    let steps = if scenario.record_steps {
        recorder.steps()
    } else {
        Vec::new()
    };
    Ok(assemble(outcome, calls, steps, logs.lines()))
}

// the fakes behind B969's access, B970's budget and rate checks and B971's counter
struct Parts {
    settings: Settings,
    checks: ScriptedChecks,
    budgets: ScriptedBudgets,
    hooks: ScriptedHooks,
    counter: ScriptedCounter,
    repositories: NoRows,
}

impl Parts {
    fn new(scenario: &Scenario, recorder: Recorder) -> Self {
        let settings = Settings {
            // ast-grep-ignore: no-long-names
            context_management_summary_model: scenario.summary_model.clone(),
            // ast-grep-ignore: no-long-names
            context_management_summary_max_tokens: scenario.max_tokens,
        };
        let checks = ScriptedChecks {
            recorder: recorder.clone(),
            verdict: scenario.policy.access,
        };
        let budgets = ScriptedBudgets {
            recorder: recorder.clone(),
            verdict: scenario.policy.budget,
        };
        let hooks = ScriptedHooks {
            recorder,
            limiter: ScriptedLimiter {
                verdict: scenario.policy.rate,
            },
        };
        let counter = ScriptedCounter {
            script: scenario.counter,
        };
        Self {
            settings,
            checks,
            budgets,
            hooks,
            counter,
            repositories: NoRows,
        }
    }
}

async fn apply(
    scenario: &Scenario,
    parts: &Parts,
    routed: &Port<'_>,
    direct: &Port<'_>,
    principal: Option<&UserAPIKeyAuth>,
) -> Result<PolyfillResult, SummaryExecutionError<io::Error>> {
    let proxy = ProxyGlobals {
        database: false,
        router: None,
        checks: &parts.checks,
        repositories: &parts.repositories,
    };
    let routed_port: &dyn SummaryCompletion = routed;
    let direct_port: &dyn SummaryCompletion = direct;
    let services = EditorServices {
        settings: Some(&parts.settings),
        proxy: Some(&proxy),
        budgets: Some(&parts.budgets),
        hooks: Some(&parts.hooks),
        counter: &parts.counter,
        router: (scenario.route == Route::Router).then_some(routed_port),
        direct: direct_port,
    };
    let request = EditorRequest {
        model: &scenario.model,
        messages: &scenario.messages,
        tools: scenario.tools.as_deref(),
        system: scenario.system.as_ref(),
        edit_spec: &scenario.edit_spec,
        litellm_metadata: scenario.metadata.as_ref(),
        principal,
    };
    apply_compact_20260112(request, &services).await
}

fn assemble(
    outcome: Result<PolyfillResult, SummaryExecutionError<io::Error>>,
    calls: Vec<CallRecord>,
    steps: Vec<String>,
    diagnostics: Vec<Diagnostic>,
) -> Run {
    let (result, failure) = match outcome {
        Ok(value) => (Some(shape(value)), None),
        Err(error) => (None, Some(failure(&error))),
    };
    Run {
        result,
        failure,
        calls,
        steps,
        diagnostics,
    }
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

// only the trigger error belongs to the case; the rest are wiring failures no transport state reaches
fn failure(error: &SummaryExecutionError<io::Error>) -> Failure {
    match error {
        SummaryExecutionError::Trigger(trigger) => Failure {
            status_code: trigger.status_code,
            message: trigger.message.clone(),
        },
        SummaryExecutionError::Access(_)
        | SummaryExecutionError::Budget(_)
        | SummaryExecutionError::Rate(_)
        | SummaryExecutionError::History(_)
        | SummaryExecutionError::Extraction(_)
        | SummaryExecutionError::Principal(_)
        | SummaryExecutionError::Encode(_) => Failure {
            status_code: 0,
            message: error.to_string(),
        },
    }
}

fn principal(fields: &Object) -> Result<UserAPIKeyAuth, SdkHttpError> {
    let raw = serde_json::to_value(fields).map_err(SdkHttpError::Serialize)?;
    let typed: BTreeMap<String, JsonData> =
        serde_json::from_value(raw).map_err(SdkHttpError::Serialize)?;
    UserAPIKeyAuth::construct(AuthInput::Dict(typed)).map_err(|error| {
        SdkHttpError::Invalid(format!("the case principal does not build: {error}"))
    })
}

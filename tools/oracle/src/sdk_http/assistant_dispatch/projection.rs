use betterlite_core::api::assistant_run_sdk::service::RunOutcome;
use betterlite_error::envelope::WireField;
use betterlite_http::proxy::ProxiedClient;
use betterlite_openai::thread_runs::stream::RunStream;
use betterlite_server::api::assistant_dispatch::error::AssistantDispatchError;
use betterlite_server::api::assistant_dispatch::outcome::AssistantOutcome;

use super::super::error::SdkHttpError;
use super::super::result::{Failure, Nullable};
use super::model::Step;

// A successful wrapper call carries the ids it returned and, for a polled run, its status.
const fn record(ids: Vec<String>, status: Option<String>) -> Step {
    Step {
        ids,
        status,
        events: None,
        failure: None,
    }
}

// A wrapper call that raised reports no ids, status or events.
fn failed_step(failure: Failure) -> Step {
    Step {
        ids: Vec::new(),
        status: None,
        events: None,
        failure: Some(failure),
    }
}

// The factory's own exceptions carry no status, type, param or code.
fn named(class: &str, message: String) -> Failure {
    Failure {
        class: class.to_owned(),
        status: 0,
        kind: None,
        param: None,
        code: None,
        message,
    }
}

// None: key absent, Some(None): JSON null, Some(Some(text)): text
fn nullable(field: WireField) -> Nullable {
    match field {
        WireField::Missing(_) => None,
        WireField::Null => Some(None),
        WireField::Text(text) => Some(Some(text)),
    }
}

pub(super) async fn project(
    answer: Result<AssistantOutcome<RunStream<ProxiedClient>>, AssistantDispatchError>,
) -> Result<Step, SdkHttpError> {
    match answer {
        Ok(AssistantOutcome::Run(RunOutcome::Streaming(stream))) => drained(stream).await,
        Ok(outcome) => {
            let (ids, status) = answered(outcome);
            Ok(record(ids, status))
        }
        Err(error) => dropped(error),
    }
}

// entering the manager starts the request, so a consumed stream has issued it by the time it returns
async fn drained(stream: RunStream<ProxiedClient>) -> Result<Step, SdkHttpError> {
    let consumed = stream
        .consume()
        .await
        .map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
    Ok(Step {
        ids: Vec::new(),
        status: None,
        events: Some(
            consumed
                .iter()
                .map(|event| event.name().to_owned())
                .collect(),
        ),
        failure: None,
    })
}

// The ids each non-streaming answer returns, and the status a polled run carries.
fn answered(outcome: AssistantOutcome<RunStream<ProxiedClient>>) -> (Vec<String>, Option<String>) {
    match outcome {
        AssistantOutcome::Assistant(assistant) => (vec![assistant.id], None),
        AssistantOutcome::Assistants(page) => (
            page.data
                .into_iter()
                .map(|assistant| assistant.id)
                .collect(),
            None,
        ),
        AssistantOutcome::Deleted(deleted) => (vec![deleted.id], None),
        AssistantOutcome::Thread(thread) => (
            thread
                .map(|thread_record| thread_record.id)
                .into_iter()
                .collect(),
            None,
        ),
        AssistantOutcome::Message(message) => (vec![message.id], None),
        AssistantOutcome::Messages(page) => (
            page.data.into_iter().map(|message| message.id).collect(),
            None,
        ),
        AssistantOutcome::Run(RunOutcome::Polled(reply)) => {
            (*reply).as_ref().map_or((Vec::new(), None), |run| {
                (vec![run.id.clone()], Some(run.status.clone()))
            })
        }
        AssistantOutcome::Run(RunOutcome::Streaming(_)) => (Vec::new(), None),
    }
}

// The factory's own errors have no status, type, param or code; the async wrapper's classes do.
pub(super) fn dropped(error: AssistantDispatchError) -> Result<Step, SdkHttpError> {
    let failure = match error {
        AssistantDispatchError::Exception { message } => named("Exception", message),
        AssistantDispatchError::KeyError { key } => named("KeyError", format!("'{key}'")),
        AssistantDispatchError::TypeError { message }
        | AssistantDispatchError::Invalid { message } => named("TypeError", message),
        AssistantDispatchError::Failure { failure } => {
            let Some(status) = failure.status else {
                return Err(SdkHttpError::Invalid(format!(
                    "the dispatcher raised {} without a status code",
                    failure.class
                )));
            };
            Failure {
                class: failure.class,
                status,
                kind: nullable(failure.kind),
                param: nullable(failure.param),
                code: nullable(failure.code),
                message: failure.message,
            }
        }
        other @ (AssistantDispatchError::ReadDelete(_)
        | AssistantDispatchError::Message(_)
        | AssistantDispatchError::Run(_)
        | AssistantDispatchError::Thread { .. }) => {
            return Err(SdkHttpError::Invalid(format!(
                "the dispatcher answered with {other}"
            )));
        }
    };
    Ok(failed_step(failure))
}

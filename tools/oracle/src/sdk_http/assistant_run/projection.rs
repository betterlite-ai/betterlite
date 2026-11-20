use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use betterlite_azure::assistants::error::AssistantsRunError;
use betterlite_azure::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use betterlite_azure::credentials::source::{
    CallableTokenProvider, CredentialSources, SourceFailure,
};
use betterlite_azure::oidc_exchange::OidcExchangeInput;
use betterlite_config::sections::settings::JsonValue;
use betterlite_core::api::assistant_run_sdk::error::{
    AssistantRunError, RunDispatchError as _, RunFailure,
};
use betterlite_error::envelope::WireField;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use betterlite_openai::thread_runs::error::ThreadRunsError;
use betterlite_openai::thread_runs::records::{RunEvent, RunReply};
use betterlite_openai::thread_runs::stream::{RunEventHandler, RunStream};
use http::HeaderMap;
use secrecy::SecretString;

use super::super::native::failure as sdk_failure;
use super::super::result::{Failure, Nullable};
use super::{HandlerView, LastError, Outcome, ResponseView, RunView, StreamView};

pub(super) fn run_outcome(reply: Option<&RunReply>) -> Outcome {
    let run = reply.map(|reply| RunView {
        id: reply.id.clone(),
        status: reply.status.clone(),
        tool_calls: reply
            .required_action
            .as_ref()
            .and_then(|action| action.pointer("/submit_tool_outputs/tool_calls"))
            .and_then(JsonValue::as_array)
            .map(Vec::len),
        last_error: reply.last_error.as_ref().map(|error| LastError {
            code: error
                .get("code")
                .and_then(JsonValue::as_str)
                .map(str::to_owned),
            message: error
                .get("message")
                .and_then(JsonValue::as_str)
                .map(str::to_owned),
        }),
    });
    Outcome {
        run,
        stream: None,
        failure: None,
        response: None,
    }
}

// entering the manager starts the request, so a consumed stream has issued it by the time it returns
pub(super) async fn stream_outcome(
    stream: RunStream<ProxiedClient>,
    recorder: Option<&RecordingHandler>,
) -> Outcome {
    match stream.consume().await {
        Ok(events) => Outcome {
            run: None,
            stream: Some(StreamView {
                events: events.iter().map(|event| event.name().to_owned()).collect(),
                handler: recorder.map(handler_view),
            }),
            failure: None,
            response: None,
        },
        Err(error) => failure_outcome(&AssistantRunError::Dispatch(AssistantsRunError::Runs(
            error,
        ))),
    }
}

fn handler_view(recorder: &RecordingHandler) -> HandlerView {
    let events = recorder
        .events
        .lock()
        .map_or_else(|_poisoned| Vec::new(), |seen| seen.clone());
    HandlerView {
        events,
        ended: recorder.ended.load(Ordering::Acquire),
    }
}

// every failure the wrappers raise, projected as the litellm exception class Python reports
pub(super) fn failure_outcome(error: &AssistantRunError) -> Outcome {
    match error {
        AssistantRunError::Unsupported { .. } => unsupported(error),
        AssistantRunError::DuplicateStream => plain("TypeError", error.to_string()),
        AssistantRunError::Timeout(_)
        | AssistantRunError::InvalidTimeout { .. }
        | AssistantRunError::InvalidTimeoutText { .. }
        | AssistantRunError::InvalidExtraBody => plain("ValueError", error.to_string()),
        AssistantRunError::Dispatch(error) => raw_failure(error),
        AssistantRunError::Mapped(failure) => mapped_failure(failure),
    }
}

// litellm's BadRequestError also carries the httpx response it was raised with
fn unsupported(error: &AssistantRunError) -> Outcome {
    let mut value = named(
        "BadRequestError",
        400,
        Some(None),
        format!("litellm.BadRequestError: {error}"),
    );
    value.response = error.response().map(|(content, method)| ResponseView {
        content: content.to_owned(),
        method: method.to_owned(),
    });
    value
}

fn raw_failure(error: &AssistantsRunError) -> Outcome {
    let Some((status, body)) = error.status_and_body() else {
        return plain("Exception", error.message());
    };
    let response = UpstreamResponse {
        status,
        headers: HeaderMap::new(),
        body: body.to_vec(),
    };
    match sdk_failure("", &response) {
        Ok(failure) => outcome(failure),
        Err(unprojected) => plain("Exception", unprojected.to_string()),
    }
}

// exception_type raises a litellm class whose openai base leaves type, param and code null
fn mapped_failure(failure: &RunFailure) -> Outcome {
    let metadata = failure.class.metadata();
    outcome(Failure {
        class: failure.class_name.to_owned(),
        status: metadata.status_code,
        kind: wire(metadata.error_type),
        param: wire(metadata.param),
        code: Some(metadata.openai_code),
        message: failure.message.clone(),
    })
}

fn wire(field: WireField) -> Nullable {
    match field {
        WireField::Missing(_) => None,
        WireField::Null => Some(None),
        WireField::Text(text) => Some(Some(text)),
    }
}

// a litellm-mapped failure carries type, param and code as explicit nulls; a local one leaves them out
fn named(class: &str, status: u16, field: Nullable, message: String) -> Outcome {
    outcome(Failure {
        class: class.to_owned(),
        status,
        kind: field.clone(),
        param: field.clone(),
        code: field,
        message,
    })
}

fn plain(class: &str, message: String) -> Outcome {
    named(class, 0, None, message)
}

fn outcome(failure: Failure) -> Outcome {
    Outcome {
        run: None,
        stream: None,
        failure: Some(failure),
        response: None,
    }
}

// the callbacks a supplied handler observes; a manager entered without one records nothing
#[derive(Clone, Default)]
pub(super) struct RecordingHandler {
    pub(super) events: Arc<Mutex<Vec<String>>>,
    pub(super) ended: Arc<AtomicBool>,
}

impl RunEventHandler for RecordingHandler {
    fn on_event(&self, event: &RunEvent) {
        if let Ok(mut seen) = self.events.lock() {
            seen.push(event.name().to_owned());
        }
    }

    fn on_exception(&self, _error: &ThreadRunsError) {}

    fn on_timeout(&self) {}

    fn on_end(&self) {
        self.ended.store(true, Ordering::Release);
    }
}

// B543 owns the Azure credential sources; the run cases authenticate through keys and tokens alone
pub(super) struct NoSources;

fn unavailable(kind: &str) -> SourceFailure {
    SourceFailure {
        class: String::from("ValueError"),
        status: None,
        message: format!("no {kind} source is configured"),
    }
}

impl CredentialSources for NoSources {
    fn client_secret<'source>(
        &'source self,
        _record: &ClientSecretRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(unavailable("client secret"))
    }

    fn username_password<'source>(
        &'source self,
        _record: &UsernamePasswordRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(unavailable("username and password"))
    }

    fn sdk_refresh<'source>(
        &'source self,
        _scope: &str,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(unavailable("SDK refresh"))
    }

    fn default_credential<'source>(
        &'source self,
        _scope: &'source str,
    ) -> Option<Arc<dyn CallableTokenProvider + 'source>> {
        None
    }

    fn oidc(&self, _input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        Err(unavailable("OIDC"))
    }
}

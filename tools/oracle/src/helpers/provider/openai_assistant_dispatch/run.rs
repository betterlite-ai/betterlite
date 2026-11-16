use std::sync::Arc;
use std::time::Duration;

use betterlite_config::tls::Verification;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_openai::assistants::client::ClientOptions;
use betterlite_openai::assistants::records::AssistantCreate;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::realtime::logging::{RealtimeLogRecord, RealtimeLogger};
use betterlite_openai::thread_runs::operations::RunDispatch;
use betterlite_openai::threads::records::{ThreadCreate, ThreadMessageData};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_provider::openai_assistant_dispatch::OpenAiAssistantRequest;
use betterlite_provider::openai_assistant_dispatch::call::{
    AssistantsCall, AssistantsFacade, AssistantsOperation, DecisionsCall, DecisionsOperation,
    OpenAiAssistantOperation, RealtimeCall, RealtimeFailureOwned, RunsCall, ThreadsCall,
    ThreadsOperation,
};
use betterlite_provider::openai_assistant_dispatch::execute::dispatch as compose;
use betterlite_provider::operation::{OperationRequest, OperationResult};
use secrecy::SecretString;
use serde::de::DeserializeOwned;
use serde_json::json;
use tokio::runtime::Builder;

use super::input::{Input, Kind, Operation, SslVerification};
use super::invalid_input;
use super::project::{failure, outcome};
use super::wire::Wire;

const ORG: &str = "org-fixture";
const FAILURE: &str = "stop";

// The composition logs nothing itself, so the oracle's logger only has to accept the pre-call record.
struct Quiet;

impl RealtimeLogger for Quiet {
    fn pre_call(&mut self, _record: RealtimeLogRecord<'_>) -> Result<(), PrePostError> {
        Ok(())
    }
}

pub(crate) fn invoke(input: &Input) -> HelperResult {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| invalid_input(error.to_string()))?;
    let operation = operation(input)?;
    let wire = Wire::replying(&input.replies);
    runtime.block_on(exchange(operation, wire))
}

async fn exchange(operation: OpenAiAssistantOperation, wire: Arc<Wire>) -> HelperResult {
    let reader = Arc::clone(&wire);
    let mut logger = Quiet;
    let result = compose(
        OperationRequest::OpenAiAssistant(OpenAiAssistantRequest),
        operation,
        wire,
        &mut logger,
    )
    .await;
    let requests = reader.seen();
    match result {
        Ok(OperationResult::OpenAiAssistant(reply)) => {
            let answer = outcome(reply).await?;
            Ok(json!({"requests": requests, "result": answer}))
        }
        Ok(_other) => Err(invalid_input(String::from(
            "the composition returned another operation",
        ))),
        Err(error) => Err(failure(&error)),
    }
}

fn operation(input: &Input) -> Result<OpenAiAssistantOperation, Box<HelperError>> {
    match input.kind {
        Kind::Assistants => assistants(input),
        Kind::Threads => threads(input),
        Kind::Runs => runs(input),
        Kind::Decisions => decisions(input),
        Kind::Realtime => realtime(input),
    }
}

fn options(input: &Input) -> ClientOptions {
    ClientOptions {
        api_key: input.api_key.clone().map(SecretString::from),
        api_base: input.api_base.clone(),
        timeout: Some(Duration::from_secs(30)),
        max_retries: Some(0),
        organization: Some(String::from(ORG)),
    }
}

fn required(field: Option<&str>, name: &str) -> Result<String, Box<HelperError>> {
    field
        .map(str::to_owned)
        .ok_or_else(|| invalid_input(format!("{name} is required")))
}

fn from_data<T: DeserializeOwned>(input: &Input, name: &str) -> Result<T, Box<HelperError>> {
    let data = input
        .data
        .clone()
        .ok_or_else(|| invalid_input(format!("{name} is required")))?;
    serde_json::from_value(data).map_err(|error| invalid_input(error.to_string()))
}

fn wrong_operation(input: &Input, family: &str) -> Box<HelperError> {
    invalid_input(format!(
        "{:?} is not an operation of the {family} family",
        input.operation
    ))
}

fn assistants(input: &Input) -> Result<OpenAiAssistantOperation, Box<HelperError>> {
    let operation = match input.operation {
        Operation::AsyncList => AssistantsOperation::List {
            facade: match input.aget_assistants {
                None => AssistantsFacade::AsyncHelper,
                Some(aget_assistants) => AssistantsFacade::Facade { aget_assistants },
            },
            params: input.params.list_params(),
        },
        Operation::Create => AssistantsOperation::Create {
            data: Box::new(from_data::<AssistantCreate>(input, "data")?),
            async_requested: true,
        },
        Operation::Delete => AssistantsOperation::Delete {
            assistant_id: required(input.assistant_id.as_deref(), "assistant_id")?,
            async_requested: true,
        },
        Operation::Retrieve
        | Operation::AddMessage
        | Operation::ListMessages
        | Operation::Buffered
        | Operation::Stream
        | Operation::Request
        | Operation::Response
        | Operation::Connect => return Err(wrong_operation(input, "assistants")),
    };
    Ok(OpenAiAssistantOperation::Assistants(AssistantsCall {
        options: options(input),
        operation,
    }))
}

fn threads(input: &Input) -> Result<OpenAiAssistantOperation, Box<HelperError>> {
    let operation = match input.operation {
        Operation::Create => ThreadsOperation::Create {
            data: from_data::<ThreadCreate>(input, "data")?,
            async_requested: true,
        },
        Operation::Retrieve => ThreadsOperation::Retrieve {
            thread_id: required(input.thread_id.as_deref(), "thread_id")?,
            async_requested: true,
        },
        Operation::AddMessage => ThreadsOperation::AddMessage {
            thread_id: required(input.thread_id.as_deref(), "thread_id")?,
            data: Box::new(from_data::<ThreadMessageData>(input, "data")?),
            async_requested: true,
        },
        Operation::ListMessages => ThreadsOperation::ListMessages {
            thread_id: required(input.thread_id.as_deref(), "thread_id")?,
            async_requested: true,
        },
        Operation::AsyncList
        | Operation::Delete
        | Operation::Buffered
        | Operation::Stream
        | Operation::Request
        | Operation::Response
        | Operation::Connect => return Err(wrong_operation(input, "threads")),
    };
    Ok(OpenAiAssistantOperation::Threads(ThreadsCall {
        options: options(input),
        operation,
    }))
}

fn runs(input: &Input) -> Result<OpenAiAssistantOperation, Box<HelperError>> {
    let request = input
        .request
        .clone()
        .ok_or_else(|| invalid_input(String::from("request is required for a run")))?;
    let dispatch = match input.operation {
        Operation::Buffered => RunDispatch {
            arun_thread: input.arun_thread,
            stream: false,
        },
        Operation::Stream => RunDispatch {
            arun_thread: true,
            stream: true,
        },
        Operation::AsyncList
        | Operation::Create
        | Operation::Delete
        | Operation::Retrieve
        | Operation::AddMessage
        | Operation::ListMessages
        | Operation::Request
        | Operation::Response
        | Operation::Connect => return Err(wrong_operation(input, "runs")),
    };
    Ok(OpenAiAssistantOperation::Runs(RunsCall {
        options: options(input),
        dispatch,
        request,
    }))
}

fn decisions(input: &Input) -> Result<OpenAiAssistantOperation, Box<HelperError>> {
    let request = input
        .decision
        .clone()
        .ok_or_else(|| invalid_input(String::from("decision is required")))?;
    let model = required(input.model.as_deref(), "model")?;
    let operation = match input.operation {
        Operation::Request => DecisionsOperation::Request { model, request },
        Operation::Response => DecisionsOperation::Response {
            model,
            request,
            payload: required(input.payload.as_deref(), "payload")?.into_bytes(),
        },
        Operation::AsyncList
        | Operation::Create
        | Operation::Delete
        | Operation::Retrieve
        | Operation::AddMessage
        | Operation::ListMessages
        | Operation::Buffered
        | Operation::Stream
        | Operation::Connect => return Err(wrong_operation(input, "decisions")),
    };
    let globals = LiteLlmGlobals {
        api_base: input.globals.api_base.clone(),
        api_key: input.globals.api_key.clone().map(SecretString::from),
        openai_key: input.globals.openai_key.clone().map(SecretString::from),
        organization: None,
    };
    Ok(OpenAiAssistantOperation::Decisions(DecisionsCall {
        api_base: input.api_base.clone(),
        api_key: input.api_key.clone().map(SecretString::from),
        globals,
        environment: input.environment.clone(),
        operation,
    }))
}

fn verification(kind: SslVerification) -> Verification {
    match kind {
        SslVerification::Enabled => Verification::Enabled,
        SslVerification::Disabled => Verification::Disabled,
    }
}

fn realtime(input: &Input) -> Result<OpenAiAssistantOperation, Box<HelperError>> {
    let call = match &input.close_error {
        Some(close_error) => RealtimeCall::Failure(RealtimeFailureOwned::CloseFailed {
            message: String::from(FAILURE),
            close_error: close_error.clone(),
        }),
        None => RealtimeCall::Connect {
            api_base: input.api_base.clone(),
            api_key: input.api_key.clone().map(SecretString::from),
            model: required(input.model.as_deref(), "model")?,
            query_params: input.query_params.clone(),
            beta_realtime: input.openai_beta,
            ssl_verification: verification(input.ssl_verification),
            max_message_size_bytes: input.max_message_size_bytes.ok_or_else(|| {
                invalid_input(String::from(
                    "max_message_size_bytes is required for a realtime connect",
                ))
            })?,
        },
    };
    Ok(OpenAiAssistantOperation::Realtime(call))
}

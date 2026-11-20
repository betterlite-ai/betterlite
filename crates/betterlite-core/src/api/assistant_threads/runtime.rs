use std::future::Future;
use std::sync::Arc;

use betterlite_azure::azure_thread_messages::error::ThreadMessagesError;
use betterlite_azure::azure_thread_messages::operations::{
    ThreadMessagesApi, ThreadOperation as AzureOperation,
};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::assistants::transport::AssistantsTransport;
use betterlite_openai::threads::client as openai_threads;
use betterlite_openai::threads::error::ThreadsError;
use betterlite_openai::threads::operations::{ThreadCall, ThreadReply};
use betterlite_openai::threads::records::ThreadRecord;

use super::error::{ThreadError, provider_failure};
use super::target::{Operation, PreparedThread, Target};

// the provider call, behind a seam so the pass-through of a null SDK result is testable
pub(super) trait ThreadRuntime<T> {
    fn run(
        &self,
        prepared: PreparedThread<'_, T>,
    ) -> impl Future<Output = Result<Option<ThreadRecord>, ThreadError>> + Send;
}

pub(super) struct SdkThreadRuntime;

impl<T: AssistantsTransport> ThreadRuntime<T> for SdkThreadRuntime {
    fn run(
        &self,
        prepared: PreparedThread<'_, T>,
    ) -> impl Future<Output = Result<Option<ThreadRecord>, ThreadError>> + Send {
        let PreparedThread { target, operation } = prepared;
        async move {
            match target {
                Target::Openai(client) => {
                    let call = match &operation {
                        Operation::Create(data) => ThreadCall::Create(data),
                        Operation::Retrieve(thread_id) => ThreadCall::Retrieve(thread_id),
                    };
                    openai_reply(openai_threads::send(&client, call).await)
                }
                Target::Azure {
                    request,
                    client,
                    flag,
                } => {
                    let azure_operation = match &operation {
                        Operation::Create(data) => AzureOperation::Create(data),
                        Operation::Retrieve(thread_id) => AzureOperation::Retrieve(thread_id),
                    };
                    let api = ThreadMessagesApi {
                        transport: Arc::clone(&client.transport),
                    };
                    let reply = api
                        .call(&request, Some(client), flag, azure_operation)
                        .await
                        .map_err(azure_error)?;
                    Ok(Some(reply.thread))
                }
            }
        }
    }
}

fn openai_reply(
    reply: Result<ThreadReply, ThreadsError>,
) -> Result<Option<ThreadRecord>, ThreadError> {
    match reply {
        Ok(ThreadReply::Thread(thread)) => Ok(Some(thread)),
        Ok(ThreadReply::Message(_) | ThreadReply::Page(_)) => Err(ThreadError::Reply),
        Err(error) => Err(map_threads_error(error)),
    }
}

fn azure_error(error: ThreadMessagesError) -> ThreadError {
    match error {
        ThreadMessagesError::Threads(threads) => map_threads_error(threads),
        ThreadMessagesError::Client(client) => ThreadError::Definitions(client),
    }
}

fn map_threads_error(error: ThreadsError) -> ThreadError {
    match error {
        ThreadsError::Status { status, body } => {
            let failure = provider_failure(status, &body);
            let message = format!("Error code: {status} - {}", failure.body_text);
            ThreadError::Provider {
                message,
                failure: Box::new(failure),
            }
        }
        timeout @ ThreadsError::Transport {
            source: TransportError::Deadline(_) | TransportError::ConnectTimeout(_),
        } => ThreadError::ProviderTimeout(timeout),
        other @ (ThreadsError::MissingApiKey
        | ThreadsError::BaseUrl { .. }
        | ThreadsError::Header { .. }
        | ThreadsError::EmptyThreadId
        | ThreadsError::DotSegment { .. }
        | ThreadsError::Encode { .. }
        | ThreadsError::Transport { .. }
        | ThreadsError::Reply { .. }) => ThreadError::Threads(other),
    }
}

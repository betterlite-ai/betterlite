use std::sync::Arc;
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::RequestBody;
use tokio::time::sleep;

use crate::assistants::client::{ClientKind, ClientOptions, SdkClient};
use crate::assistants::operations::AssistantsApi;
use crate::assistants::transport::AssistantsTransport;
use crate::thread_runs::client::{CallKind, outbound, send};
use crate::thread_runs::decode::decode_json;
use crate::thread_runs::error::ThreadRunsError;
use crate::thread_runs::records::{RunCreate, RunReply, RunRequest, RunStatus};
use crate::thread_runs::stream::{RunEventHandler, RunStream};

pub struct RunsApi<T> {
    pub transport: Arc<T>,
}

// Both run tests compare with `is True`, so only a literal true takes the async or streaming branch.
#[derive(Debug, Clone, Copy, Default)]
pub struct RunDispatch {
    pub arun_thread: Option<bool>,
    pub stream: Option<bool>,
}

pub struct RunCall<T> {
    pub options: ClientOptions,
    pub client: Option<SdkClient<T>>,
    pub dispatch: RunDispatch,
    pub handler: Option<Arc<dyn RunEventHandler>>,
}

// The buffered and streaming results stay distinct, and each names the facade it was built for.
pub enum RunInvocation<T> {
    Polled {
        kind: ClientKind,
        run: Box<RunReply>,
    },
    Streaming(Box<RunStream<T>>),
}

impl<T: AssistantsTransport> RunsApi<T> {
    pub async fn create_and_poll(
        &self,
        client: &SdkClient<T>,
        request: &RunRequest,
    ) -> Result<RunReply, ThreadRunsError> {
        let segments = thread_segments(&request.thread_id)?;
        let body = RunCreate {
            assistant_id: &request.assistant_id,
            additional_instructions: request.additional_instructions.as_deref(),
            instructions: request.instructions.as_deref(),
            metadata: request.metadata.as_ref(),
            model: request.model.as_deref(),
            stream: false,
            tools: request.tools.as_deref(),
        };
        let bytes =
            serde_json::to_vec(&body).map_err(|source| ThreadRunsError::Encode { source })?;
        let create = outbound(
            client,
            CallKind::Create,
            &segments,
            RequestBody::Json(bytes),
        )?;
        let created = send(self.transport.as_ref(), create).await?;
        let mut run = decode_json::<RunReply>(&created.body)?;
        loop {
            let poll_segments = run_segments(&request.thread_id, &run.id)?;
            let poll = outbound(client, CallKind::Poll, &poll_segments, RequestBody::Empty)?;
            let response = send(self.transport.as_ref(), poll).await?;
            run = decode_json::<RunReply>(&response.body)?;
            if matches!(
                run.status,
                RunStatus::RequiresAction
                    | RunStatus::Cancelled
                    | RunStatus::Completed
                    | RunStatus::Failed
                    | RunStatus::Expired
                    | RunStatus::Incomplete
            ) {
                return Ok(run);
            }
            sleep(poll_interval(&response)?).await;
        }
    }

    pub fn create_stream(
        &self,
        client: &SdkClient<T>,
        request: &RunRequest,
        handler: Option<Arc<dyn RunEventHandler>>,
    ) -> Result<RunStream<T>, ThreadRunsError> {
        let segments = thread_segments(&request.thread_id)?;
        let body = RunCreate {
            assistant_id: &request.assistant_id,
            additional_instructions: request.additional_instructions.as_deref(),
            instructions: request.instructions.as_deref(),
            metadata: request.metadata.as_ref(),
            model: request.model.as_deref(),
            stream: true,
            tools: request.tools.as_deref(),
        };
        let bytes =
            serde_json::to_vec(&body).map_err(|source| ThreadRunsError::Encode { source })?;
        let kind = CallKind::Stream {
            custom_handler: handler.is_some(),
        };
        let stream_request = outbound(client, kind, &segments, RequestBody::Json(bytes))?;
        Ok(RunStream {
            kind: client.kind,
            handler,
            transport: Arc::clone(&self.transport),
            request: stream_request,
        })
    }

    pub async fn run_thread(
        &self,
        call: RunCall<T>,
        request: &RunRequest,
    ) -> Result<RunInvocation<T>, ThreadRunsError> {
        let kind = if call.dispatch.arun_thread == Some(true) {
            ClientKind::Async
        } else {
            ClientKind::Sync
        };
        let client = AssistantsApi {
            transport: Arc::clone(&self.transport),
        }
        .get_openai_client(kind, call.options, call.client);
        if call.dispatch.stream == Some(true) {
            let stream = self.create_stream(&client, request, call.handler)?;
            return Ok(RunInvocation::Streaming(Box::new(stream)));
        }
        let run = self.create_and_poll(&client, request).await?;
        Ok(RunInvocation::Polled {
            kind,
            run: Box::new(run),
        })
    }
}

fn thread_segments(thread_id: &str) -> Result<[&str; 3], ThreadRunsError> {
    check_segment(thread_id, ThreadRunsError::EmptyThreadId)?;
    Ok(["threads", thread_id, "runs"])
}

fn run_segments<'call>(
    thread_id: &'call str,
    run_id: &'call str,
) -> Result<[&'call str; 4], ThreadRunsError> {
    check_segment(thread_id, ThreadRunsError::EmptyThreadId)?;
    if run_id.is_empty() {
        return Err(ThreadRunsError::EmptyRunId);
    }
    if matches!(run_id, "." | "..") {
        return Err(ThreadRunsError::DotSegment {
            segment: run_id.to_owned(),
        });
    }
    Ok(["threads", thread_id, "runs", run_id])
}

fn check_segment(segment: &str, empty: ThreadRunsError) -> Result<(), ThreadRunsError> {
    if segment.is_empty() {
        return Err(empty);
    }
    if matches!(segment, "." | "..") {
        return Err(ThreadRunsError::DotSegment {
            segment: segment.to_owned(),
        });
    }
    Ok(())
}

fn poll_interval(response: &UpstreamResponse) -> Result<Duration, ThreadRunsError> {
    let Some(value) = response.headers.get("openai-poll-after-ms") else {
        return Ok(Duration::from_millis(1000));
    };
    let text = value
        .to_str()
        .map_err(|source| ThreadRunsError::PollAfterHeader { source })?;
    let millis = text
        .parse::<u64>()
        .map_err(|source| ThreadRunsError::PollAfter {
            value: text.to_owned(),
            source,
        })?;
    Ok(Duration::from_millis(millis))
}

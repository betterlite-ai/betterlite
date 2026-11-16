use std::sync::Arc;

use betterlite_openai::assistants::operations::AssistantsApi;
use betterlite_openai::assistants::transport::AssistantsTransport;
use betterlite_openai::decisions::openai::config::OpenAiDecisionsConfig;
use betterlite_openai::decisions::openai::request::openai_request_to_ir;
use betterlite_openai::decisions::openai::response::to_openai::ir_to_openai_response;
use betterlite_openai::decisions::transformation::provider_config::DecisionsProviderConfig;
use betterlite_openai::realtime::connect::{RealtimeConnectSettings, prepare_connection};
use betterlite_openai::realtime::failure::client_action;
use betterlite_openai::realtime::logging::RealtimeLogger;
use betterlite_openai::thread_runs::operations::{RunCall, RunInvocation, RunsApi};
use betterlite_openai::thread_runs::stream::RunStreamTransport;
use betterlite_openai::threads::operations::{ClientSetup, ThreadCall, ThreadReply, ThreadsApi};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use secrecy::{ExposeSecret as _, SecretString};

use crate::operation::{OperationRequest, OperationResult};

use super::call::{
    AssistantsCall, AssistantsFacade, AssistantsOperation, DecisionsCall, DecisionsOperation,
    RealtimeCall, RunsCall, ThreadsCall, ThreadsOperation,
};
use super::error::OpenAiAssistantError;
use super::reply::{AssistantsReply, DecisionsProjection, DecisionsReply, RunOutcome};
use super::{
    OpenAiAssistantPrepared, OpenAiAssistantRequest, OpenAiAssistantResult, SessionNormalizerKind,
};

// The closed provider enum is the only entry: every other operation stays outside this composition.
pub async fn dispatch<T, L>(
    request: OperationRequest,
    transport: Arc<T>,
    logger: &mut L,
) -> Result<OperationResult, OpenAiAssistantError>
where
    T: AssistantsTransport + RunStreamTransport + 'static,
    L: RealtimeLogger,
{
    let OperationRequest::OpenAiAssistant(request) = request else {
        return Err(OpenAiAssistantError::UnsupportedOperation);
    };
    let result = match request {
        OpenAiAssistantRequest::Assistants(call) => {
            OpenAiAssistantResult::Assistants(assistants(call, &transport).await?)
        }
        OpenAiAssistantRequest::Threads(call) => {
            OpenAiAssistantResult::Threads(threads(call, &transport).await?)
        }
        OpenAiAssistantRequest::Runs(call) => {
            OpenAiAssistantResult::Runs(runs(call, &transport).await?)
        }
        OpenAiAssistantRequest::Decisions(call) => {
            OpenAiAssistantResult::Decisions(decisions(call)?)
        }
        OpenAiAssistantRequest::Realtime(call) => session(call, logger)?,
    };
    Ok(OperationResult::OpenAiAssistant(result))
}

async fn assistants<T>(
    call: AssistantsCall,
    transport: &Arc<T>,
) -> Result<AssistantsReply, OpenAiAssistantError>
where
    T: AssistantsTransport,
{
    let api = AssistantsApi {
        transport: Arc::clone(transport),
    };
    let reply = match call.operation {
        AssistantsOperation::List { facade, params } => AssistantsReply::Page(match facade {
            AssistantsFacade::AsyncHelper => {
                api.async_get_assistants(call.options, None, &params).await?
            }
            AssistantsFacade::Facade { aget_assistants } => {
                api.get_assistants(call.options, None, aget_assistants, &params)
                    .await?
            }
        }),
        AssistantsOperation::Create {
            data,
            async_requested,
        } => AssistantsReply::Assistant(
            api.create_assistants(call.options, &data, None, async_requested)
                .await?,
        ),
        AssistantsOperation::Delete {
            assistant_id,
            async_requested,
        } => AssistantsReply::Deleted(
            api.delete_assistant(call.options, &assistant_id, None, async_requested)
                .await?,
        ),
    };
    Ok(reply)
}

async fn threads<T>(
    call: ThreadsCall,
    transport: &Arc<T>,
) -> Result<ThreadReply, OpenAiAssistantError>
where
    T: AssistantsTransport,
{
    let api = ThreadsApi {
        assistants: AssistantsApi {
            transport: Arc::clone(transport),
        },
    };
    let reply = match call.operation {
        ThreadsOperation::Create {
            data,
            async_requested,
        } => {
            let setup = ClientSetup {
                options: call.options,
                supplied: None,
                async_requested,
            };
            api.call(setup, ThreadCall::Create(&data)).await?
        }
        ThreadsOperation::Retrieve {
            thread_id,
            async_requested,
        } => {
            let setup = ClientSetup {
                options: call.options,
                supplied: None,
                async_requested,
            };
            api.call(setup, ThreadCall::Retrieve(&thread_id)).await?
        }
        ThreadsOperation::AddMessage {
            thread_id,
            data,
            async_requested,
        } => {
            let setup = ClientSetup {
                options: call.options,
                supplied: None,
                async_requested,
            };
            let request = ThreadCall::AddMessage {
                thread_id: &thread_id,
                data: &data,
            };
            api.call(setup, request).await?
        }
        ThreadsOperation::ListMessages {
            thread_id,
            async_requested,
        } => {
            let setup = ClientSetup {
                options: call.options,
                supplied: None,
                async_requested,
            };
            api.call(setup, ThreadCall::ListMessages(&thread_id)).await?
        }
    };
    Ok(reply)
}

async fn runs<T>(call: RunsCall, transport: &Arc<T>) -> Result<RunOutcome, OpenAiAssistantError>
where
    T: AssistantsTransport + RunStreamTransport + 'static,
{
    let api = RunsApi {
        transport: Arc::clone(transport),
    };
    let run = RunCall {
        options: call.options,
        client: None,
        dispatch: call.dispatch,
        handler: None,
    };
    let invocation = api.run_thread(run, &call.request).await?;
    let outcome = match invocation {
        RunInvocation::Polled(kind, reply) => RunOutcome::Polled(kind, reply),
        RunInvocation::Streaming(stream) => RunOutcome::Streaming(stream.events().await?),
    };
    Ok(outcome)
}

fn decisions(call: DecisionsCall) -> Result<DecisionsReply, OpenAiAssistantError> {
    let config = OpenAiDecisionsConfig::default();
    let api_base = config.resolve_api_base(call.api_base.as_deref(), |_name| String::new());
    let supplied_key = call.api_key.as_ref().map(|key| key.expose_secret().as_str());
    let api_key = config
        .resolve_api_key(supplied_key, |_name| String::new())
        .map(SecretString::from);
    let projection = match call.operation {
        DecisionsOperation::Request { model, request } => {
            let ir = openai_request_to_ir(&request);
            DecisionsProjection::Request(config.transform_decisions_request(
                &model, &ir, "openai",
            )?)
        }
        DecisionsOperation::Response {
            model,
            request,
            payload,
        } => {
            let ir = openai_request_to_ir(&request);
            let response = config.parse_response(&payload, &ir)?;
            DecisionsProjection::Response(Box::new(ir_to_openai_response(
                &response, &ir, &model,
            )?))
        }
    };
    Ok(DecisionsReply {
        api_base,
        api_key,
        projection,
    })
}

fn session<L: RealtimeLogger>(
    call: RealtimeCall,
    logger: &mut L,
) -> Result<OpenAiAssistantResult, OpenAiAssistantError> {
    match call {
        RealtimeCall::Connect {
            api_base,
            api_key,
            model,
            query_params,
            beta_realtime,
            ssl_verification,
            max_message_size_bytes,
        } => {
            let settings = RealtimeConnectSettings {
                api_base: api_base.as_deref(),
                api_key: api_key.as_ref(),
                model: &model,
                query_params: query_params.as_ref(),
                beta_realtime,
                ssl_verification: &ssl_verification,
                max_message_size_bytes,
            };
            let plan = prepare_connection(&settings, logger)?;
            let event_normalizer = if plan.event_normalizer.is_some() {
                SessionNormalizerKind::Installed
            } else {
                SessionNormalizerKind::Passthrough
            };
            Ok(OpenAiAssistantResult::Session(OpenAiAssistantPrepared {
                url: plan.url,
                headers: plan.headers,
                ssl: plan.ssl,
                max_message_size_bytes: plan.max_message_size_bytes,
                force_transcription_model: plan.force_transcription_model,
                event_normalizer,
            }))
        }
        RealtimeCall::Failure(failure) => {
            let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)?;
            let action = client_action(failure.borrowed(), &redactor)?;
            Ok(OpenAiAssistantResult::SessionFailure(action))
        }
    }
}

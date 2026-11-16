use std::sync::Arc;

use betterlite_openai::assistants::operations::AssistantsApi;
use betterlite_openai::assistants::transport::AssistantsTransport;
use betterlite_openai::decisions::openai::config::{Credential, OpenAiDecisionsConfig};
use betterlite_openai::decisions::openai::request::openai_request_to_ir;
use betterlite_openai::decisions::openai::response::to_openai::ir_to_openai_response;
use betterlite_openai::decisions::transformation::provider_config::DecisionsProviderConfig as _;
use betterlite_openai::realtime::connect::{RealtimeConnectSettings, prepare_connection};
use betterlite_openai::realtime::failure::client_action;
use betterlite_openai::realtime::logging::RealtimeLogger;
use betterlite_openai::thread_runs::operations::{RunCall, RunInvocation, RunsApi};
use betterlite_openai::thread_runs::stream::RunStreamTransport;
use betterlite_openai::threads::operations::{ClientSetup, ThreadsApi};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use futures_util::stream::once;
use futures_util::{StreamExt as _, TryStreamExt as _};
use secrecy::{ExposeSecret, SecretString};

use crate::operation::{OperationRequest, OperationResult};

use super::call::{
    AssistantsCall, AssistantsFacade, AssistantsOperation, DecisionsCall, DecisionsOperation,
    OpenAiAssistantOperation, RealtimeCall, RunsCall,
};
use super::error::OpenAiAssistantError;
use super::reply::{AssistantsReply, DecisionsProjection, DecisionsReply, RunOutcome};
use super::{OpenAiAssistantPrepared, OpenAiAssistantRequest, OpenAiAssistantResult};

// The closed provider enum is the only entry: every other operation stays outside this composition, and the typed call names the family.
pub async fn dispatch<T, L>(
    request: OperationRequest,
    operation: OpenAiAssistantOperation,
    transport: Arc<T>,
    logger: &mut L,
) -> Result<OperationResult, OpenAiAssistantError>
where
    T: AssistantsTransport + RunStreamTransport + 'static,
    L: RealtimeLogger + Send,
{
    let OperationRequest::OpenAiAssistant(OpenAiAssistantRequest) = request else {
        return Err(OpenAiAssistantError::UnsupportedOperation);
    };
    let result = match operation {
        OpenAiAssistantOperation::Assistants(call) => {
            let api = AssistantsApi { transport };
            OpenAiAssistantResult::Assistants(Box::new(assistants(&api, call).await?))
        }
        OpenAiAssistantOperation::Threads(call) => {
            let setup = ClientSetup {
                options: call.options,
                supplied: None,
                async_requested: call.async_requested,
            };
            let api = ThreadsApi {
                assistants: AssistantsApi { transport },
            };
            let thread_call = call.operation.leaf_call();
            OpenAiAssistantResult::Threads(Box::new(api.call(setup, thread_call).await?))
        }
        OpenAiAssistantOperation::Runs(call) => {
            OpenAiAssistantResult::Runs(Box::new(runs(&RunsApi { transport }, call).await?))
        }
        OpenAiAssistantOperation::Decisions(call) => {
            OpenAiAssistantResult::Decisions(Box::new(decisions(call)?))
        }
        OpenAiAssistantOperation::Realtime(call) => session(call, logger)?,
    };
    Ok(OperationResult::OpenAiAssistant(result))
}

async fn assistants<T>(
    api: &AssistantsApi<T>,
    AssistantsCall { options, operation }: AssistantsCall,
) -> Result<AssistantsReply, OpenAiAssistantError>
where
    T: AssistantsTransport,
{
    Ok(match operation {
        AssistantsOperation::List {
            facade: AssistantsFacade::AsyncHelper,
            params,
        } => AssistantsReply::Page(api.async_get_assistants(options, None, &params).await?),
        AssistantsOperation::List {
            facade: AssistantsFacade::Facade { aget_assistants },
            params,
        } => AssistantsReply::Page(
            api.get_assistants(options, None, aget_assistants, &params)
                .await?,
        ),
        AssistantsOperation::Create {
            data,
            async_requested,
        } => AssistantsReply::Assistant(Box::new(
            api.create_assistants(options, &data, None, async_requested)
                .await?,
        )),
        AssistantsOperation::Delete {
            assistant_id,
            async_requested,
        } => AssistantsReply::Deleted(
            api.delete_assistant(options, &assistant_id, None, async_requested)
                .await?,
        ),
    })
}

async fn runs<T>(api: &RunsApi<T>, call: RunsCall) -> Result<RunOutcome, OpenAiAssistantError>
where
    T: AssistantsTransport + RunStreamTransport + 'static,
{
    let run = RunCall {
        options: call.options,
        client: None,
        dispatch: call.dispatch,
        handler: None,
    };
    let invocation = api.run_thread(run, &call.request).await?;
    Ok(match invocation {
        RunInvocation::Polled(kind, reply) => RunOutcome::Polled(kind, reply),
        // LiteLLM hands back the SDK stream manager unopened, so the POST goes out on the caller's first poll and an open failure is the first item
        RunInvocation::Streaming(stream) => {
            RunOutcome::Streaming(stream.kind, once(stream.events()).try_flatten().boxed())
        }
    })
}

fn decisions(call: DecisionsCall) -> Result<DecisionsReply, OpenAiAssistantError> {
    let DecisionsCall {
        api_base: explicit_base,
        api_key: explicit_key,
        globals,
        environment,
        operation,
    } = call;
    let config = OpenAiDecisionsConfig { globals };
    let get_secret_str = |name: &str| environment.get(name).cloned().unwrap_or_default();
    let api_base = config.resolve(explicit_base.as_deref(), Credential::Base, &get_secret_str);
    let supplied_key = explicit_key.as_ref().map(ExposeSecret::expose_secret);
    let api_key = config
        .resolve(supplied_key, Credential::Key, &get_secret_str)
        .map(SecretString::from);
    let projection = match operation {
        DecisionsOperation::Request { model, request } => {
            let ir = openai_request_to_ir(&request);
            DecisionsProjection::Request(Box::new(
                config.transform_decisions_request(&model, &ir, "openai")?,
            ))
        }
        DecisionsOperation::Response {
            model,
            request,
            payload,
        } => {
            let ir = openai_request_to_ir(&request);
            let response = config.parse_response(&payload, &ir)?;
            DecisionsProjection::Response(Box::new(ir_to_openai_response(&response, &ir, &model)?))
        }
    };
    Ok(DecisionsReply {
        api_base,
        api_key,
        projection,
    })
}

fn session<L>(
    call: RealtimeCall,
    logger: &mut L,
) -> Result<OpenAiAssistantResult, OpenAiAssistantError>
where
    L: RealtimeLogger,
{
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
            Ok(OpenAiAssistantResult::Session(Box::new(
                OpenAiAssistantPrepared {
                    url: plan.url,
                    headers: plan.headers,
                    ssl: plan.ssl,
                    max_message_size_bytes: plan.max_message_size_bytes,
                    force_transcription_model: plan.force_transcription_model,
                    event_normalizer_installed: plan.event_normalizer.is_some(),
                },
            )))
        }
        RealtimeCall::Failure(failure) => {
            let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)?;
            let action = client_action(failure.borrowed(), &redactor)?;
            Ok(OpenAiAssistantResult::SessionFailure(Box::new(action)))
        }
    }
}

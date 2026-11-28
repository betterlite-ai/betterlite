use super::super::completion::{CompletionCall, HandlerFuture, ResolvedCall};
use super::legacy::Transport;
use super::{ClientHandle, Counter, Failure, Native, Reply, Runtime, config::Config};
use betterlite_anthropic::chat_call::transport::ChatTransport;
use betterlite_anthropic::chat_call::{
    CallStyle, ChatCall, ChatClients, ChatDeps, ChatOutcome, chat_call,
};
use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_http::client_kind::SuppliedClient;
use betterlite_logging::logging_pre_post::context::LogContext;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use betterlite_prompts::anthropic_results::tool_result::FileBlocks;
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_tokenizer::engine::Engine;
use secrecy::ExposeSecret as _;
use std::error::Error as StdError;
use std::sync::{Mutex, PoisonError};

fn execute<'call, Client, FileFailure>(
    call: &'call CompletionCall<ClientHandle<Client>, Engine>,
    config: &'call Config<'_>,
    runtime: &'call Runtime<'_, Client>,
    log: &'call mut LogContext<'_, '_>,
) -> HandlerFuture<
    'call,
    ChatOutcome<Client::Failure, Counter>,
    Failure<Client::Failure, FileFailure>,
>
where
    Client: ChatTransport,
    FileFailure: StdError + Send + Sync + 'static,
{
    Box::pin(async move {
        let native = ChatCall {
            model: &call.model,
            custom_llm_provider: &call.provider,
            messages: call.messages.clone(),
            optional_params: call.optional_params.clone(),
            litellm_params: call.litellm_params.clone(),
            caller_headers: call
                .headers
                .iter()
                .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
                .collect(),
            api_base: call.api_base.as_deref().ok_or(Failure::MissingBase)?,
            api_key: call.api_key.as_ref(),
            settings: runtime.settings,
            timeout: call.timeout,
            style: if call.acompletion {
                CallStyle::Async
            } else {
                CallStyle::Sync
            },
            effort_used: runtime.effort_used,
            now: runtime.now.timestamp(),
            disable_add_prefix_to_prompt: runtime.disable_add_prefix_to_prompt,
            model_response: call.model_response.clone(),
        };
        let deps = runtime.deps(call, config, log);
        chat_call(native, deps)
            .await
            .map_err(|source| Failure::Chat(Box::new(source)))
    })
}

impl<Client> Runtime<'_, Client> {
    fn deps<'call, 'matrix>(
        &'call self,
        call: &'call CompletionCall<ClientHandle<Client>, Engine>,
        config: &'call Config<'matrix>,
        log: &'call mut LogContext<'_, '_>,
    ) -> ChatDeps<'call, Config<'matrix>, Client, Counter> {
        let clients = ChatClients {
            pooled: self.pooled,
            supplied: call.client.as_ref().map(|client| SuppliedClient {
                kind: client.kind,
                transport: &client.transport,
            }),
        };
        ChatDeps {
            config,
            clients,
            state: log.state,
            env: log.env,
            stream_logger: self.stream_logger,
            counter: Counter(call.encoding.clone()),
            ids: self.ids,
            max_event_bytes: self.max_event_bytes,
            max_error_body_bytes: self.max_error_body_bytes,
        }
    }
}

type Completion<'call, ClientFailure, FileFailure> =
    HandlerFuture<'call, Reply<ClientFailure>, Failure<ClientFailure, FileFailure>>;

pub(super) fn complete<'call, Client, Source, Metadata, Fetcher, Blocks, Ids>(
    handler: &'call mut Native<'_, Client, Source, Metadata, Fetcher, Blocks, Ids>,
    call: ResolvedCall<ClientHandle<Client>, Engine>,
    log: &'call mut LogContext<'_, '_>,
) -> Completion<'call, Client::Failure, Blocks::Failure>
where
    Client: ChatTransport + Transport,
    Source: DynamicMetadataSource,
    Metadata: ModelInfoClient,
    Fetcher: RemoteMediaFetcher + Sync,
    Blocks: FileBlocks + Sync,
    Ids: Fn() -> String + Sync,
{
    Box::pin(async move {
        let mut request = call.call;
        if call.config != Some(ChatConfigTag::Anthropic) {
            return Err(Failure::Config);
        }
        let mut config = Config {
            validated: Mutex::new(None),
            prepared: None,
            matrix: handler.runtime.matrix,
        };
        let first = execute(&request, &config, &handler.runtime, log).await;
        let Some(validated) = config
            .validated
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        else {
            return first.map(Reply::Messages);
        };
        let prepared = handler
            .preparation
            .prepare(&request.model, &request.provider, validated)
            .await
            .map_err(|source| Failure::Prepare(Box::new(source)))?;
        config.install(prepared, &mut request.litellm_params)?;
        execute(&request, &config, &handler.runtime, log)
            .await
            .map(Reply::Messages)
    })
}

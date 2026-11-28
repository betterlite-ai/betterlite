use betterlite_anthropic::response::wire::ResponseEnvelope;
use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::time::Duration;

use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_logging::logging_pre_post::args::AdditionalArgs;
use betterlite_logging::logging_pre_post::context::LogContext;
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_models::decisions::Json;
use futures_util::future::BoxFuture;
use indexmap::IndexMap;
use secrecy::SecretString;

use super::DispatchError;
use super::base::{Environment, Globals, Surface, resolve_base, resolve_key};
use super::config::selected_config;

// Client and encoding handles stay in the caller's types across the native handler boundary.
#[derive(Debug, Clone)]
pub struct CompletionCall<Client, Encoding> {
    pub model: String,
    pub provider: String,
    pub messages: Vec<Json>,
    pub model_response: ResponseEnvelope,
    pub optional_params: IndexMap<String, Json>,
    pub litellm_params: IndexMap<String, Json>,
    pub headers: BTreeMap<String, SecretString>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub custom_prompt_dict: BTreeMap<String, Json>,
    pub stream: bool,
    pub acompletion: bool,
    pub client: Option<Client>,
    pub shared_session: Option<Client>,
    pub encoding: Encoding,
    pub timeout: Duration,
}

pub struct CompletionSettings<'settings> {
    pub globals: &'settings Globals,
    pub environment: &'settings Environment,
    pub custom_prompt_dict: &'settings BTreeMap<String, Json>,
    pub disable_suffix: bool,
    pub selection: Option<ChatConfigTag>,
}

pub struct ResolvedCall<Client, Encoding> {
    pub call: CompletionCall<Client, Encoding>,
    pub surface: Surface,
    pub config: Option<ChatConfigTag>,
}

pub type HandlerFuture<'call, Reply, Failure> = BoxFuture<'call, Result<Reply, Failure>>;

// B584's chat call and B010's legacy HTTP call are supplied at their native call seam.
pub trait CompletionHandler<Client, Encoding> {
    type Reply;
    type Failure: StdError + Send + Sync + 'static;

    fn complete<'call>(
        &'call mut self,
        call: ResolvedCall<Client, Encoding>,
        log: &'call mut LogContext<'_, '_>,
    ) -> HandlerFuture<'call, Self::Reply, Self::Failure>;

    fn logged_response(reply: &Self::Reply) -> Result<Json, Self::Failure>;
}

pub fn complete<'call, Client, Encoding, Handler>(
    surface: Surface,
    mut call: CompletionCall<Client, Encoding>,
    settings: &'call CompletionSettings<'_>,
    handler: &'call mut Handler,
    log: &'call mut LogContext<'_, '_>,
) -> HandlerFuture<'call, Handler::Reply, DispatchError<Handler::Failure>>
where
    Client: Send + 'call,
    Encoding: Send + 'call,
    Handler: CompletionHandler<Client, Encoding> + Send,
{
    Box::pin(async move {
        let config = resolve(surface, &mut call, settings).map_err(DispatchError::Selection)?;
        let post = surface == Surface::Messages
            && (call.acompletion
                || matches!(call.optional_params.get("stream"), Some(Json::Bool(true))));
        let input = Json::List(call.messages.clone());
        let reply = handler
            .complete(
                ResolvedCall {
                    call,
                    surface,
                    config,
                },
                log,
            )
            .await
            .map_err(DispatchError::Completion)?;
        if post {
            post_call(
                log.state,
                &PostCall {
                    original_response: Handler::logged_response(&reply)
                        .map_err(DispatchError::Completion)?,
                    input,
                    additional_args: AdditionalArgs::default(),
                },
                log.env,
            );
        }
        Ok(reply)
    })
}

fn resolve<Client, Encoding>(
    surface: Surface,
    call: &mut CompletionCall<Client, Encoding>,
    settings: &CompletionSettings<'_>,
) -> Result<Option<ChatConfigTag>, super::config::ConfigSelectionError> {
    call.api_key = resolve_key(
        call.api_key.as_ref(),
        settings.globals,
        settings.environment,
    );
    let base = resolve_base(
        surface,
        call.api_base.as_deref(),
        settings.globals,
        settings.environment,
        settings.disable_suffix,
    );
    if let Some(notice) = base.notice {
        tracing::debug!("{notice}");
    }
    call.api_base = Some(base.url);
    let config = match surface {
        Surface::Messages => {
            call.custom_prompt_dict
                .clone_from(settings.custom_prompt_dict);
            call.shared_session = None;
            Some(selected_config(
                &call.model,
                &call.provider,
                settings.selection,
            )?)
        }
        Surface::Legacy => {
            "anthropic_text".clone_into(&mut call.provider);
            call.custom_prompt_dict.clear();
            call.client = None;
            None
        }
    };
    Ok(config)
}

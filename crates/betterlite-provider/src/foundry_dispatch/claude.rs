use std::error::Error as StdError;
use std::time::Duration;

use betterlite_anthropic::chat_call::config::ChatConfig;
use betterlite_anthropic::chat_call::transport::ChatTransport;
use betterlite_anthropic::chat_call::{ChatDeps, ChatOutcome};
use betterlite_anthropic::response::wire::ResponseEnvelope;
use betterlite_anthropic::usage::ReasoningTokenCounter;
use betterlite_foundry::anthropic::error::ChatError;
use betterlite_foundry::anthropic::handler::{CompletionResult, completion};
use betterlite_foundry::anthropic::request::{ChatRequest, Style};
use betterlite_foundry::anthropic_messages::environment::{ParamsKey, TokenSource};
use betterlite_models::decisions::Json;
use betterlite_prompts::arguments::json::Object as PromptObject;
use indexmap::IndexMap;
use secrecy::SecretString;
use thiserror::Error;

use super::credentials::{Globals, resolve_key};
use super::post_call::{self, FoundryLogger};
use super::route;
use crate::openai_dispatch::logging::{LoggedCall, logged_json};

const ANTHROPIC_PATH: &str = "/anthropic";
const MESSAGES_PATH: &str = "/v1/messages";

// The rewrite is literal string handling: the first `/anthropic` keeps its prefix and loses the rest
#[must_use]
pub fn endpoint(base: &str) -> String {
    if base.is_empty() {
        return String::new();
    }
    let trimmed = base.trim_end_matches('/');
    if trimmed.ends_with(MESSAGES_PATH) {
        return trimmed.to_owned();
    }
    let head = trimmed
        .split_once(ANTHROPIC_PATH)
        .map_or(trimmed, |(prefix, _rest)| prefix);
    format!("{head}{ANTHROPIC_PATH}{MESSAGES_PATH}")
}

pub struct Call<'data> {
    pub model: &'data str,
    pub messages: &'data [PromptObject],
    pub optional_params: IndexMap<String, Json>,
    pub litellm_params: &'data PromptObject,
    pub caller_headers: IndexMap<String, String>,
    pub api_base: Option<&'data str>,
    pub api_key: Option<&'data SecretString>,
    pub timeout: Duration,
    pub style: Style,
    pub stream: bool,
    pub now: i64,
    pub disable_add_prefix_to_prompt: bool,
}

// The resolved base, key and parameter map the handler borrows for one call
pub struct Prepared {
    url: String,
    argument_key: Option<SecretString>,
    messages: Vec<Json>,
    optional_params: IndexMap<String, Json>,
    litellm_params: IndexMap<String, Json>,
}

#[derive(Debug, Error)]
pub enum PrepareError {
    #[error("{}", route::CLAUDE_BASE_MESSAGE)]
    MissingBase,
    #[error("the call could not be converted for the Anthropic handler: {source}")]
    Convert {
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Debug, Error)]
pub enum DispatchError<ClientFailure, CounterFailure>
where
    ClientFailure: StdError + Send + Sync + 'static,
    CounterFailure: StdError + Send + Sync + 'static,
{
    #[error(transparent)]
    Prepare(#[from] PrepareError),
    #[error(transparent)]
    Handler(#[from] ChatError<ClientFailure, CounterFailure>),
}

pub fn prepare(call: &Call<'_>, globals: &Globals<'_>) -> Result<Prepared, PrepareError> {
    let base = globals
        .accessors()
        .api_base(call.api_base.filter(|text| !text.is_empty()))
        .ok_or(PrepareError::MissingBase)?;
    Ok(Prepared {
        url: endpoint(&base),
        // main.py resolves the key through B549 before the handler builds its headers
        argument_key: resolve_key(call.api_key, globals),
        messages: serde_json::to_value(call.messages)
            .and_then(serde_json::from_value)
            .map_err(|source| PrepareError::Convert { source })?,
        optional_params: call.optional_params.clone(),
        litellm_params: serde_json::to_value(call.litellm_params)
            .and_then(serde_json::from_value)
            .map_err(|source| PrepareError::Convert { source })?,
    })
}

// Every field borrows the prepared call, so the handler's future cannot outlive it
#[must_use]
pub fn request<'borrow, 'data>(
    prepared: &'borrow Prepared,
    call: &'borrow Call<'data>,
    globals: &'borrow Globals<'data>,
) -> ChatRequest<'borrow> {
    ChatRequest {
        model: call.model,
        messages: prepared.messages.clone(),
        optional_params: prepared.optional_params.clone(),
        litellm_params: prepared.litellm_params.clone(),
        caller_headers: call.caller_headers.clone(),
        api_base: &prepared.url,
        params_key: ParamsKey::DictAbsent,
        argument_key: prepared.argument_key.as_ref(),
        configured_key: None,
        token: TokenSource {
            credentials: globals.credentials,
            refresh_enabled: globals.refresh_enabled,
            provider: globals.token_provider.clone(),
            sources: globals.sources,
        },
        timeout: call.timeout,
        style: call.style,
        now: call.now,
        disable_add_prefix_to_prompt: call.disable_add_prefix_to_prompt,
        model_response: ResponseEnvelope::new(String::from("chatcmpl-foundry")),
    }
}

pub struct Dispatch<'call, Config, Client, Counter> {
    pub call: Call<'call>,
    pub globals: Globals<'call>,
    pub deps: ChatDeps<'call, Config, Client, Counter>,
}

impl<'call, Config, Client, Counter> Dispatch<'call, Config, Client, Counter>
where
    Config: ChatConfig,
    Client: ChatTransport,
    Counter: ReasoningTokenCounter + Unpin + Send + 'call,
{
    pub async fn run(
        self,
        logger: &mut dyn FoundryLogger,
    ) -> Result<
        ChatOutcome<Client::Failure, Counter>,
        DispatchError<Client::Failure, Counter::Failure>,
    > {
        let Self {
            call,
            globals,
            deps,
        } = self;
        let prepared = prepare(&call, &globals)?;
        let result: CompletionResult<Client, Counter> =
            completion(request(&prepared, &call, &globals), deps).await;
        let outcome = result.map_err(DispatchError::Handler)?;
        // this branch's post_call is not wrapped in a try, so a handler error reaches the caller unlogged
        if call.stream || call.style == Style::Async {
            logger.post_call(record(&call, report(&outcome)));
        }
        Ok(outcome)
    }
}

fn record(call: &Call<'_>, report: Json) -> LoggedCall {
    LoggedCall {
        input: logged_json(&call.messages),
        original_response: report,
        // main.py passes no additional_args here, so the failure log carries no headers map
        headers: None,
    }
}

fn report<Failure, Counter>(outcome: &ChatOutcome<Failure, Counter>) -> Json
where
    Failure: StdError,
    Counter: ReasoningTokenCounter,
{
    match outcome {
        ChatOutcome::Reply(envelope) => post_call::reply_report(
            envelope.model.as_deref(),
            envelope
                .choices
                .first()
                .and_then(|choice| choice.message.content.as_deref()),
        ),
        ChatOutcome::Stream(_) => {
            Json::Object(IndexMap::from([(String::from("stream"), Json::Bool(true))]))
        }
        // a chat call cannot answer with an embedding; main.py never logs one here
        ChatOutcome::Embedding => post_call::reply_report(None, None),
    }
}

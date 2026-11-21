use std::collections::BTreeMap;

use betterlite_foundry::agent_results::reply::{AgentReply, TokenCounter};
use betterlite_foundry::agent_results::request::StreamRequest;
use betterlite_foundry::agent_runs::dispatch::{
    DispatchCall, EntraResolver, Route, dispatch as dispatch_run,
};
use betterlite_foundry::agent_runs::error::AgentRunError;
use betterlite_foundry::agent_runs::flow::completion;
use betterlite_foundry::agent_runs::transport::{AgentRunClientFactory, Sleeper};
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::arguments::json::Object;
use secrecy::SecretString;
use thiserror::Error;

use super::credentials::{Globals, headers_or_global, resolve_base, resolve_key, secret_headers};
use super::route;

pub struct Call<'data> {
    pub model: &'data str,
    pub messages: &'data [Object],
    pub optional_params: &'data Object,
    pub litellm_params: &'data Object,
    pub headers: Option<&'data Headers>,
    pub api_base: Option<&'data str>,
    pub api_key: Option<&'data SecretString>,
    pub acompletion: bool,
    pub stream: bool,
    pub reply: AgentReply,
}

pub struct Deps<'data> {
    pub factory: &'data dyn AgentRunClientFactory,
    pub sleeper: &'data dyn Sleeper,
    pub counter: &'data dyn TokenCounter,
    pub resolver: &'data dyn EntraResolver,
}

pub struct Dispatch<'data> {
    pub call: Call<'data>,
    pub globals: Globals<'data>,
    pub deps: Deps<'data>,
}

pub enum Outcome {
    Reply(Box<AgentReply>),
    Stream(Box<StreamRequest>),
}

#[derive(Debug, Error)]
pub enum DispatchError {
    #[error("{}", route::AGENTS_BASE_MESSAGE)]
    MissingBase,
    #[error(transparent)]
    Run(#[from] AgentRunError),
}

// What main.py hands the agent config: the resolved base and key and the headers it falls back on
pub struct Resolved {
    pub api_base: String,
    pub api_key: Option<SecretString>,
    pub headers: BTreeMap<String, SecretString>,
}

// What main.py hands the agent config before it is called: the two resolved values and the headers
pub fn resolve(call: &Call<'_>, globals: &Globals<'_>) -> Result<Resolved, DispatchError> {
    let api_base = resolve_base(call.api_base, globals).ok_or(DispatchError::MissingBase)?;
    Ok(Resolved {
        api_base,
        api_key: resolve_key(call.api_key, globals),
        headers: secret_headers(headers_or_global(call.headers, globals)),
    })
}

impl Dispatch<'_> {
    pub fn resolve(&self) -> Result<Resolved, DispatchError> {
        resolve(&self.call, &self.globals)
    }

    // This branch has no post_call in main.py, so an agent failure reaches the caller unlogged
    pub async fn run(&self) -> Result<Outcome, DispatchError> {
        let Resolved {
            api_base,
            api_key,
            headers,
        } = self.resolve()?;
        let route = dispatch_run(
            DispatchCall {
                model: self.call.model,
                messages: self.call.messages,
                optional_params: self.call.optional_params,
                litellm_params: self.call.litellm_params,
                headers: &headers,
                environment: self.globals.environment,
                api_base: Some(&api_base),
                api_key: api_key.as_ref(),
                reply: self.call.reply.clone(),
                acompletion: self.call.acompletion,
                stream: self.call.stream,
            },
            self.deps.resolver,
        )?;
        Ok(match route {
            Route::Completion(run) => Outcome::Reply(Box::new(
                completion(
                    &run,
                    self.deps.factory,
                    self.deps.sleeper,
                    self.deps.counter,
                )
                .await?,
            )),
            Route::Stream(request) => Outcome::Stream(Box::new(request)),
        })
    }
}

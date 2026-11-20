use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use betterlite_http::interactions::base::Params;
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use secrecy::SecretString;
use uuid::Uuid;

use self::prepare::{prepare_create, prepare_get};
use self::send::{create_async, create_blocking, get_async, get_blocking};
use super::clients::{CharacterClients, SuppliedClients};
use super::config::CharacterConfig;
use super::error::{CharacterError, CharacterFault};
use crate::api::bfl::edit::media::SuppliedMedia;
use crate::api::video_records::CharacterObject;

mod prepare;
mod send;
pub(crate) mod wire;

pub struct CharacterCreateCall<'call> {
    pub name: &'call str,
    pub video: &'call SuppliedMedia,
    pub litellm_params: &'call Params,
    pub extra_headers: Option<&'call BTreeMap<String, String>>,
    pub timeout: Option<Duration>,
    pub api_key: Option<&'call SecretString>,
    pub is_async: bool,
}

// timeout is here because the public call takes one; the lookup never forwards it to the GET
pub struct CharacterGetCall<'call> {
    pub character_id: &'call str,
    pub litellm_params: &'call Params,
    pub extra_headers: Option<&'call BTreeMap<String, String>>,
    pub timeout: Option<Duration>,
    pub api_key: Option<&'call SecretString>,
    pub is_async: bool,
}

// the sync handler answers with the result, or with the pending async call when _is_async asked for one
pub enum CharacterHandled<Output, Pending> {
    Done(Output),
    Pending(Pending),
}

pub type CharacterPending<'call, Config> =
    Pin<Box<dyn Future<Output = Result<CharacterObject, CharacterFault<Config>>> + Send + 'call>>;

pub type CharacterAnswer<'call, Config> = Result<
    CharacterHandled<CharacterObject, CharacterPending<'call, Config>>,
    CharacterFault<Config>,
>;

// the four handlers' shared dependencies: the config, the client source, the logging state and its environment
pub struct CharacterTransport<'deps, 'env, C, S>
where
    C: CharacterConfig,
    S: CharacterClients,
{
    pub config: &'deps C,
    pub clients: &'deps mut S,
    pub logging: &'deps mut LoggingState,
    pub env: &'deps PrePostEnv<'env>,
    pub supplied: SuppliedClients<'deps, S>,
    pub boundary: fn() -> String,
}

impl<'deps, 'env, C, S> CharacterTransport<'deps, 'env, C, S>
where
    C: CharacterConfig,
    S: CharacterClients,
{
    // a built client carries the uuid boundary httpx generates for a multipart body
    #[must_use]
    pub fn with_generated_boundary(
        config: &'deps C,
        clients: &'deps mut S,
        logging: &'deps mut LoggingState,
        env: &'deps PrePostEnv<'env>,
    ) -> Self {
        Self {
            config,
            clients,
            logging,
            env,
            supplied: SuppliedClients {
                sync: None,
                asynchronous: None,
            },
            boundary: || Uuid::new_v4().simple().to_string(),
        }
    }

    // a caller's clients by the class each one is; a sync call ignores an async-only client, as isinstance does
    #[must_use]
    pub fn supplying(mut self, supplied: SuppliedClients<'deps, S>) -> Self {
        self.supplied = supplied;
        self
    }

    #[must_use]
    pub fn video_create_character_handler<'call>(
        &'call mut self,
        call: &'call CharacterCreateCall<'_>,
    ) -> CharacterAnswer<'call, C> {
        if call.is_async {
            let pending = self.async_video_create_character_handler(call);
            return Ok(CharacterHandled::Pending(Box::pin(pending)));
        }
        let client = self
            .clients
            .select_sync(call.litellm_params, self.supplied.sync)
            .map_err(CharacterError::Client)?;
        let request = prepare_create(
            self.config,
            self.logging,
            self.env,
            &(self.boundary)(),
            call,
        )?;
        create_blocking(self.config, self.logging, &client, request).map(CharacterHandled::Done)
    }

    #[must_use]
    // ast-grep-ignore: no-long-names
    pub async fn async_video_create_character_handler(
        &mut self,
        call: &CharacterCreateCall<'_>,
    ) -> Result<CharacterObject, CharacterFault<C>> {
        let client = self
            .clients
            .select_async(call.litellm_params, self.supplied.asynchronous)
            .map_err(CharacterError::Client)?;
        let boundary = (self.boundary)();
        let request = prepare_create(self.config, self.logging, self.env, &boundary, call)?;
        create_async(self.config, self.logging, &client, request).await
    }

    #[must_use]
    pub fn video_get_character_handler<'call>(
        &'call mut self,
        call: &'call CharacterGetCall<'_>,
    ) -> CharacterAnswer<'call, C> {
        if call.is_async {
            let pending = self.async_video_get_character_handler(call);
            return Ok(CharacterHandled::Pending(Box::pin(pending)));
        }
        let client = self
            .clients
            .select_sync(call.litellm_params, self.supplied.sync)
            .map_err(CharacterError::Client)?;
        let request = prepare_get(self.config, self.logging, self.env, call)?;
        get_blocking(self.config, self.logging, &client, request).map(CharacterHandled::Done)
    }

    #[must_use]
    // ast-grep-ignore: no-long-names
    pub async fn async_video_get_character_handler(
        &mut self,
        call: &CharacterGetCall<'_>,
    ) -> Result<CharacterObject, CharacterFault<C>> {
        let client = self
            .clients
            .select_async(call.litellm_params, self.supplied.asynchronous)
            .map_err(CharacterError::Client)?;
        let request = prepare_get(self.config, self.logging, self.env, call)?;
        get_async(self.config, self.logging, &client, request).await
    }
}

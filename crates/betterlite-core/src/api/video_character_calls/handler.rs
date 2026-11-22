use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use betterlite_http::interactions::base::Params;
use betterlite_logging::logging_state::LoggingState;
use thiserror::Error;

use super::error::{CharacterHandlerFailure, CharacterProviderFailure};
use crate::api::bfl::edit::media::SuppliedMedia;
use crate::api::video_character_transport::clients::CharacterClients;
use crate::api::video_character_transport::config::CharacterConfig;
use crate::api::video_character_transport::error::{CharacterError, CharacterFault};
use crate::api::video_character_transport::service::{
    CharacterCreateCall, CharacterGetCall, CharacterHandled, CharacterTransport,
};
use crate::api::video_records::CharacterObject;

// the future a handler raises for the async form of the call
pub type CharacterWork<'call> =
    Pin<Box<dyn Future<Output = Result<CharacterObject, CharacterHandlerFailure>> + Send + 'call>>;

// the finished character, or the future the async form resolves
pub enum CharacterAnswered<'call> {
    Done(CharacterObject),
    Pending(CharacterWork<'call>),
}

// what a create handler is handed: the primary argument, the provider, the params and the caller's keyword values
pub struct CreateHandlerCall {
    pub name: String,
    pub video: SuppliedMedia,
    pub provider: String,
    pub litellm_params: Params,
    pub extra_headers: Option<BTreeMap<String, String>>,
    pub timeout: Duration,
    pub is_async: bool,
}

// what a get handler is handed
pub struct GetHandlerCall {
    pub character_id: String,
    pub provider: String,
    pub litellm_params: Params,
    pub extra_headers: Option<BTreeMap<String, String>>,
    pub timeout: Duration,
    pub is_async: bool,
}

// the two handler calls, owned so the async answer can outlive the operation that raised it
pub trait CharacterHandler: Send {
    fn logging(&mut self) -> &mut LoggingState;

    fn create(
        &mut self,
        call: CreateHandlerCall,
    ) -> Result<CharacterAnswered<'_>, CharacterHandlerFailure>;

    fn get(
        &mut self,
        call: GetHandlerCall,
    ) -> Result<CharacterAnswered<'_>, CharacterHandlerFailure>;
}

// B3335's transport as the handler both operations run
impl<C, S> CharacterHandler for CharacterTransport<'_, '_, C, S>
where
    C: CharacterConfig,
    S: CharacterClients,
    C::Error: CharacterProviderFailure,
{
    fn logging(&mut self) -> &mut LoggingState {
        self.logging
    }

    fn create(
        &mut self,
        call: CreateHandlerCall,
    ) -> Result<CharacterAnswered<'_>, CharacterHandlerFailure> {
        // the async answer outlives the transport call, so the call is built inside the future it returns
        if call.is_async {
            let work = Box::pin(async move {
                let transport_call = create_transport_call(&call, true);
                create_answered(self, &transport_call).await
            });
            return Ok(CharacterAnswered::Pending(work));
        }
        let transport_call = create_transport_call(&call, false);
        let handled = self
            .video_create_character_handler(&transport_call)
            .map_err(fault_of::<C>)?;
        match handled {
            CharacterHandled::Done(character) => Ok(CharacterAnswered::Done(character)),
            CharacterHandled::Pending(_pending) => Err(unexpected_pending()),
        }
    }

    fn get(
        &mut self,
        call: GetHandlerCall,
    ) -> Result<CharacterAnswered<'_>, CharacterHandlerFailure> {
        if call.is_async {
            let work = Box::pin(async move {
                let transport_call = get_transport_call(&call, true);
                get_answered(self, &transport_call).await
            });
            return Ok(CharacterAnswered::Pending(work));
        }
        let transport_call = get_transport_call(&call, false);
        let handled = self
            .video_get_character_handler(&transport_call)
            .map_err(fault_of::<C>)?;
        match handled {
            CharacterHandled::Done(character) => Ok(CharacterAnswered::Done(character)),
            CharacterHandled::Pending(_pending) => Err(unexpected_pending()),
        }
    }
}

const fn create_transport_call<'call>(
    call: &'call CreateHandlerCall,
    is_async: bool,
) -> CharacterCreateCall<'call> {
    CharacterCreateCall {
        name: &call.name,
        video: &call.video,
        litellm_params: &call.litellm_params,
        extra_headers: call.extra_headers.as_ref(),
        timeout: Some(call.timeout),
        api_key: None,
        is_async,
    }
}

const fn get_transport_call<'call>(
    call: &'call GetHandlerCall,
    is_async: bool,
) -> CharacterGetCall<'call> {
    CharacterGetCall {
        character_id: &call.character_id,
        litellm_params: &call.litellm_params,
        extra_headers: call.extra_headers.as_ref(),
        timeout: Some(call.timeout),
        api_key: None,
        is_async,
    }
}

async fn create_answered<C, S>(
    transport: &mut CharacterTransport<'_, '_, C, S>,
    call: &CharacterCreateCall<'_>,
) -> Result<CharacterObject, CharacterHandlerFailure>
where
    C: CharacterConfig,
    S: CharacterClients,
    C::Error: CharacterProviderFailure,
{
    let handled = transport
        .video_create_character_handler(call)
        .map_err(fault_of::<C>)?;
    match handled {
        CharacterHandled::Done(character) => Ok(character),
        CharacterHandled::Pending(pending) => pending.await.map_err(fault_of::<C>),
    }
}

async fn get_answered<C, S>(
    transport: &mut CharacterTransport<'_, '_, C, S>,
    call: &CharacterGetCall<'_>,
) -> Result<CharacterObject, CharacterHandlerFailure>
where
    C: CharacterConfig,
    S: CharacterClients,
    C::Error: CharacterProviderFailure,
{
    let handled = transport
        .video_get_character_handler(call)
        .map_err(fault_of::<C>)?;
    match handled {
        CharacterHandled::Done(character) => Ok(character),
        CharacterHandled::Pending(pending) => pending.await.map_err(fault_of::<C>),
    }
}

// a provider fault keeps its status and message, and every other transport fault keeps its own text
fn fault_of<C>(fault: CharacterFault<C>) -> CharacterHandlerFailure
where
    C: CharacterConfig,
    C::Error: CharacterProviderFailure,
{
    let (status_code, message) = match &fault {
        CharacterError::Provider(error) => (error.status_code(), error.message()),
        CharacterError::Client(_)
        | CharacterError::Logging(_)
        | CharacterError::Multipart(_)
        | CharacterError::Request(_) => (None, fault.to_string()),
    };
    CharacterHandlerFailure {
        status_code,
        message,
        source: Box::new(fault),
    }
}

// the transport answers a blocking call with the character; a pending answer there breaks that contract
fn unexpected_pending() -> CharacterHandlerFailure {
    let message = "the blocking character handler answered with a pending future";
    CharacterHandlerFailure {
        status_code: None,
        message: message.to_owned(),
        source: Box::new(UnexpectedPending),
    }
}

#[derive(Debug, Error)]
#[error("the blocking character handler answered with a pending future")]
struct UnexpectedPending;

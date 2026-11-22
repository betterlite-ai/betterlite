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
    CharacterAnswer, CharacterCreateCall, CharacterGetCall, CharacterHandled, CharacterTransport,
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

// the primary argument the call carries: the name and video of a creation, or the id of a lookup
pub enum HandlerArguments {
    Create { name: String, video: SuppliedMedia },
    Get { character_id: String },
}

// what a handler is handed: the primary argument, the provider, the params and the caller's keyword values
pub struct HandlerCall {
    pub arguments: HandlerArguments,
    pub provider: String,
    pub litellm_params: Params,
    pub extra_headers: Option<BTreeMap<String, String>>,
    pub timeout: Duration,
    pub is_async: bool,
}

// the handler both operations run: B3335's transport, or a fixture in tests and the oracle
pub trait CharacterHandler: Send {
    fn logging(&mut self) -> &mut LoggingState;

    fn handle(
        &mut self,
        call: HandlerCall,
    ) -> Result<CharacterAnswered<'_>, CharacterHandlerFailure>;
}

// the transport call either primary argument builds
enum TransportCall<'call> {
    Create(CharacterCreateCall<'call>),
    Get(CharacterGetCall<'call>),
}

impl HandlerCall {
    const fn transport_call(&self, is_async: bool) -> TransportCall<'_> {
        match &self.arguments {
            HandlerArguments::Create { name, video } => {
                TransportCall::Create(CharacterCreateCall {
                    name: name.as_str(),
                    video,
                    litellm_params: &self.litellm_params,
                    extra_headers: self.extra_headers.as_ref(),
                    timeout: Some(self.timeout),
                    api_key: None,
                    is_async,
                })
            }
            HandlerArguments::Get { character_id } => TransportCall::Get(CharacterGetCall {
                character_id: character_id.as_str(),
                litellm_params: &self.litellm_params,
                extra_headers: self.extra_headers.as_ref(),
                timeout: Some(self.timeout),
                api_key: None,
                is_async,
            }),
        }
    }
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

    fn handle(
        &mut self,
        call: HandlerCall,
    ) -> Result<CharacterAnswered<'_>, CharacterHandlerFailure> {
        if call.is_async {
            let work = Box::pin(async move { answered(self, &call.transport_call(true)).await });
            return Ok(CharacterAnswered::Pending(work));
        }
        let transport_call = call.transport_call(false);
        match ask(self, &transport_call).map_err(fault_of::<C>)? {
            CharacterHandled::Done(character) => Ok(CharacterAnswered::Done(character)),
            CharacterHandled::Pending(_pending) => Err(unexpected_pending()),
        }
    }
}

// the transport's own handler for the call's kind
fn ask<'call, C, S>(
    transport: &'call mut CharacterTransport<'_, '_, C, S>,
    call: &'call TransportCall<'_>,
) -> CharacterAnswer<'call, C>
where
    C: CharacterConfig,
    S: CharacterClients,
    C::Error: CharacterProviderFailure,
{
    match call {
        TransportCall::Create(create_call) => transport.video_create_character_handler(create_call),
        TransportCall::Get(get_call) => transport.video_get_character_handler(get_call),
    }
}

async fn answered<C, S>(
    transport: &mut CharacterTransport<'_, '_, C, S>,
    call: &TransportCall<'_>,
) -> Result<CharacterObject, CharacterHandlerFailure>
where
    C: CharacterConfig,
    S: CharacterClients,
    C::Error: CharacterProviderFailure,
{
    match ask(transport, call).map_err(fault_of::<C>)? {
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

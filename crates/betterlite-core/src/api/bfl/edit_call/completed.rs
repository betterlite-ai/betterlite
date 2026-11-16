use std::sync::{Mutex, PoisonError};

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use http::header::HeaderMap;
use url::Url;

use super::error::EditCallError;
use crate::api::bfl::edit::transform_image_edit_response;
use crate::api::bfl::generation::BflImageResponse;
use crate::api::bfl::poll::PollTransport;

// B1518's projection reads the completed reply's bytes, which the shared poll executor keeps to itself
#[derive(Default)]
pub(super) struct Completed(Mutex<Option<(u16, Vec<u8>)>>);

impl Completed {
    pub(super) fn project(&self, now_seconds: i64) -> Result<BflImageResponse, EditCallError> {
        let Some((status, body)) = self.0.lock().unwrap_or_else(PoisonError::into_inner).take()
        else {
            return Err(EditCallError::MissingCompletedResponse);
        };
        Ok(transform_image_edit_response(status, &body, now_seconds)?)
    }
}

pub(super) struct Recording<'provider, 'completed, T> {
    pub(super) transport: &'provider T,
    pub(super) completed: &'completed Completed,
}

impl<T> PollTransport for Recording<'_, '_, T>
where
    T: PollTransport + Sync,
{
    async fn get(&self, url: Url, headers: HeaderMap) -> Result<UpstreamResponse, TransportError> {
        let response = self.transport.get(url, headers).await?;
        let finished = (response.status, response.body.clone());
        *self
            .completed
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(finished);
        Ok(response)
    }
}

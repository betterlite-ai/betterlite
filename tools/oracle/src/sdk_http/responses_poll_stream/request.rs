use std::future::pending;

use async_trait::async_trait;
use betterlite_core::api::responses_poll_stream::ports::{ClientInbound, ClientMessage, PollScope};
use betterlite_models::decisions::Json;

use super::Call;
use crate::sdk_http::error::SdkHttpError;

// detach_request_from_client's _never_receive: the worker holds the link but never reads a disconnect
pub(super) struct Paused;

#[async_trait]
impl ClientInbound for Paused {
    async fn receive(&mut self) -> ClientMessage {
        pending::<()>().await;
        ClientMessage::Disconnect
    }
}

// the preprocessed scope the admitted request carried, kept for the detached provider call
pub(super) fn scope(call: &Call) -> Result<PollScope, SdkHttpError> {
    let body =
        serde_json::to_vec(&Json::Object(call.data.clone())).map_err(SdkHttpError::Serialize)?;
    Ok(PollScope {
        method: String::from("POST"),
        path: String::from("/v1/responses"),
        headers: vec![
            (
                String::from("authorization"),
                format!("Bearer {}", call.api_key),
            ),
            (
                String::from("content-type"),
                String::from("application/json"),
            ),
        ],
        body,
    })
}

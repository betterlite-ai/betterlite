use super::SenderFuture;
use super::error::RealtimeSenderError;
use crate::api::realtime_beta_protocol::events::translate_event_to_beta;
use crate::api::realtime_contracts::transport::ClientWebSocket;
use betterlite_models::decisions::Json;
use futures_util::FutureExt as _;
use std::borrow::Cow;
use std::error::Error;

pub(super) fn event_type_label(event: &Json) -> String {
    let Json::Object(fields) = event else {
        return String::from("<non-string>");
    };
    match fields.get("type") {
        Some(Json::Text(kind)) => kind.clone(),
        _ => String::from("<non-string>"),
    }
}

pub(super) enum ClientFrame<'event> {
    Beta(&'event Json),
    Text(&'event str),
}

// Both beta delivery and fallback use the same socket boundary; only beta delivery can omit a frame.
pub(super) fn send_client_frame<'call, ConfigError, CallbackError>(
    connection: &'call mut Option<Box<dyn ClientWebSocket>>,
    frame: ClientFrame<'call>,
) -> SenderFuture<'call, bool, ConfigError, CallbackError>
where
    ConfigError: Error + 'static,
    CallbackError: Error + 'static,
{
    async move {
        let encoded = match frame {
            ClientFrame::Text(text) => Cow::Borrowed(text),
            ClientFrame::Beta(event) => {
                let outcome =
                    translate_event_to_beta(event.clone()).map_err(RealtimeSenderError::Beta)?;
                let Some(translated) = outcome else {
                    return Ok(false);
                };
                Cow::Owned(serde_json::to_string(&translated).map_err(RealtimeSenderError::Encode)?)
            }
        };
        connection
            .as_mut()
            .ok_or(RealtimeSenderError::ClientMissing)?
            .send_text(&encoded)
            .await
            .map_err(RealtimeSenderError::Client)?;
        Ok(true)
    }
    .boxed_local()
}

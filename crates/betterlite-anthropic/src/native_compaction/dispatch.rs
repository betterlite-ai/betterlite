use std::error::Error;
use std::future::Future;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::error::NativeCompactionError;
use super::protocol::{CompactionProtocol, Mapping};
use error::DispatchError;

pub mod error;

const MESSAGE_ROLES: [&str; 6] = [
    "user",
    "assistant",
    "tool",
    "system",
    "function",
    "developer",
];

pub struct ChatCall<'payload> {
    pub model: &'payload str,
    pub messages: &'payload [Json],
    pub stream: bool,
    pub options: IndexMap<&'payload str, &'payload Json>,
}

// The router adapter turns these into Router.aanthropic_messages(custom_llm_provider=None, client=None, **payload) and Router.acompletion(...).
pub trait CompactionRouter: Sync {
    type Error: Error + Send + Sync + 'static;

    fn aanthropic_messages(
        &self,
        payload: &Mapping,
    ) -> impl Future<Output = Result<Mapping, Self::Error>> + Send;

    fn acompletion(
        &self,
        call: &ChatCall<'_>,
    ) -> impl Future<Output = Result<Mapping, Self::Error>> + Send;
}

impl<'payload> ChatCall<'payload> {
    fn from_payload(payload: &'payload Mapping) -> Result<Self, NativeCompactionError> {
        let model = match payload.get("model") {
            Some(Json::Text(model)) => model,
            Some(_) => return Err(NativeCompactionError::NotText { field: "model" }),
            None => return Err(NativeCompactionError::Missing { field: "model" }),
        };
        let messages = match payload.get("messages") {
            Some(Json::List(messages)) => messages,
            Some(_) => return Err(NativeCompactionError::NotAList { field: "messages" }),
            None => return Err(NativeCompactionError::Missing { field: "messages" }),
        };
        for (index, message) in messages.iter().enumerate() {
            let valid = matches!(
                message,
                Json::Object(fields)
                    if matches!(fields.get("role"), Some(Json::Text(role)) if MESSAGE_ROLES.contains(&role.as_str()))
            );
            if !valid {
                return Err(NativeCompactionError::BadMessage { index });
            }
        }
        let options = payload
            .iter()
            .filter(|(key, _)| !matches!(key.as_str(), "model" | "messages" | "stream"))
            .map(|(key, value)| (key.as_str(), value))
            .collect();
        Ok(Self {
            model,
            messages,
            stream: false,
            options,
        })
    }
}

pub async fn dispatch<Router>(
    router: &Router,
    protocol: CompactionProtocol,
    payload: &Mapping,
) -> Result<Mapping, DispatchError<Router::Error>>
where
    Router: CompactionRouter,
{
    if protocol == CompactionProtocol::Messages {
        return router
            .aanthropic_messages(payload)
            .await
            .map_err(|source| DispatchError::Router { source });
    }
    let call = ChatCall::from_payload(payload)?;
    router
        .acompletion(&call)
        .await
        .map_err(|source| DispatchError::Router { source })
}

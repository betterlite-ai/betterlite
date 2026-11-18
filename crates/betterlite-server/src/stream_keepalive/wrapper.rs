use std::pin::Pin;
use std::time::Duration;

use betterlite_config::sections::settings::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject as StreamObject;
use futures_util::StreamExt as _;
use futures_util::stream::{Stream, unfold};
use tokio::time::timeout;

use super::JsonObject;
use super::resolver::KeepaliveResolver;

// a source carries either B8603's map or the model chunk's map, and a borrowed conversion between them cannot outlive the item
pub trait KeepaliveSource {
    fn hidden_params(&self) -> Option<&JsonObject> {
        None
    }

    fn stream_hidden_params(&self) -> Option<&StreamObject> {
        None
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Beat<T> {
    Item(T),
    Ping,
}

struct KeepaliveState<S> {
    items: Pin<Box<S>>,
    resolver: KeepaliveResolver,
    wait: Option<Duration>,
}

// A resolved interval is 0 or 1..=300, so only an interval a caller builds by hand reaches the error
fn heartbeat_wait(seconds: f64) -> Option<Duration> {
    if seconds <= 0.0_f64 {
        return None;
    }
    Duration::try_from_secs_f64(seconds).ok()
}

#[must_use]
pub fn keepalive_object(hidden: &StreamObject) -> JsonObject {
    hidden
        .iter()
        .map(|(key, value)| (key.clone(), json_value(value)))
        .collect()
}

fn json_value(value: &Json) -> JsonValue {
    match value {
        Json::Null => JsonValue::Null,
        Json::Bool(flag) => JsonValue::Bool(*flag),
        Json::Number(number) => JsonValue::Number(number.clone()),
        Json::Text(text) => JsonValue::String(text.clone()),
        Json::List(items) => JsonValue::Array(items.iter().map(json_value).collect()),
        Json::Object(fields) => JsonValue::Object(
            fields
                .iter()
                .map(|(key, field)| (key.clone(), json_value(field)))
                .collect(),
        ),
    }
}

pub fn with_keepalive<S, T>(
    items: S,
    resolver: KeepaliveResolver,
    initial_seconds: f64,
) -> impl Stream<Item = Beat<T>>
where
    S: Stream<Item = T> + Send + 'static,
    T: KeepaliveSource,
{
    let start = KeepaliveState {
        items: Box::pin(items),
        resolver,
        wait: heartbeat_wait(initial_seconds),
    };
    unfold(start, |mut state| async move {
        let item = match state.wait {
            None => state.items.next().await?,
            Some(wait) => match timeout(wait, state.items.next()).await {
                Ok(Some(item)) => item,
                Ok(None) => return None,
                Err(_) => return Some((Beat::Ping, state)),
            },
        };
        let hidden = item
            .stream_hidden_params()
            .map(keepalive_object)
            .or_else(|| item.hidden_params().cloned())
            .unwrap_or_default();
        state.wait = heartbeat_wait(state.resolver.resolve(&hidden));
        Some((Beat::Item(item), state))
    })
}

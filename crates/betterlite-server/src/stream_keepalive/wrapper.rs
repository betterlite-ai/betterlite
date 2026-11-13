use std::pin::Pin;
use std::time::Duration;

use futures_util::StreamExt as _;
use futures_util::stream::{Stream, unfold};
use tokio::time::timeout;

use super::JsonObject;
use super::resolver::KeepaliveResolver;

pub trait KeepaliveSource {
    fn hidden_params(&self) -> Option<&JsonObject>;
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
        match state.wait {
            None => {
                let item = state.items.next().await?;
                state.wait = heartbeat_wait(
                    state
                        .resolver
                        .resolve(item.hidden_params().unwrap_or(&JsonObject::new())),
                );
                Some((Beat::Item(item), state))
            }
            Some(wait) => match timeout(wait, state.items.next()).await {
                Ok(Some(item)) => {
                    state.wait = heartbeat_wait(
                        state
                            .resolver
                            .resolve(item.hidden_params().unwrap_or(&JsonObject::new())),
                    );
                    Some((Beat::Item(item), state))
                }
                Ok(None) => None,
                Err(_) => Some((Beat::Ping, state)),
            },
        }
    })
}

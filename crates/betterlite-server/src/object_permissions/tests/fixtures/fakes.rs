use std::future::{Future, ready};
use std::sync::Mutex;
use std::time::Duration;

use betterlite_auth::auth_pubsub::endpoint::{
    AuthPubSub, AuthPubSubSubscription, PubSubMessage, TransportError,
};
use betterlite_auth::auth_pubsub::eviction::{CacheError, LocalEviction};
use betterlite_auth::auth_pubsub::message::Invalidation;
use betterlite_cache::backend::BackendError;

#[derive(Default)]
pub(crate) struct RecordingEviction {
    pub(crate) keys: Mutex<Vec<String>>,
    fail: bool,
}

impl RecordingEviction {
    pub(crate) const fn failing() -> Self {
        Self {
            keys: Mutex::new(Vec::new()),
            fail: true,
        }
    }
}

impl LocalEviction for RecordingEviction {
    fn apply(&self, invalidation: Invalidation) -> Result<(), CacheError> {
        self.keys
            .lock()
            .unwrap()
            .push(invalidation.cache_key.clone());
        if !self.fail {
            return Ok(());
        }
        Err(CacheError::Eviction {
            cache_key: invalidation.cache_key,
            source: BackendError::Unsupported("b816 scripted eviction"),
        })
    }
}

#[derive(Default)]
pub(crate) struct RecordingEndpoint {
    messages: Mutex<Vec<String>>,
}

impl AuthPubSub for RecordingEndpoint {
    type Subscription = RecordingSubscription;

    fn channel(&self) -> String {
        "auth_cache_invalidation".to_owned()
    }

    fn publish(&self, message: &str) -> impl Future<Output = Result<(), TransportError>> + Send {
        self.messages.lock().unwrap().push(message.to_owned());
        ready(Ok(()))
    }

    fn subscribe(&self) -> impl Future<Output = Result<Self::Subscription, TransportError>> + Send {
        ready(Ok(RecordingSubscription))
    }
}

pub(crate) struct RecordingSubscription;

impl AuthPubSubSubscription for RecordingSubscription {
    fn next_message(
        &mut self,
        _timeout: Duration,
    ) -> impl Future<Output = Result<Option<PubSubMessage>, TransportError>> + Send {
        ready(Ok(None))
    }

    fn close(&mut self) -> impl Future<Output = Result<(), TransportError>> + Send {
        ready(Ok(()))
    }
}

// The publish runs in a task the caller owns, so the test yields until the last broadcast is visible.
pub(crate) async fn wait_for_publishes(
    endpoint: &RecordingEndpoint,
    expected: usize,
) -> Vec<String> {
    for _attempt in 0_u32..1_000_u32 {
        let messages = endpoint.messages.lock().unwrap().clone();
        if messages.len() >= expected {
            return messages;
        }
        tokio::task::yield_now().await;
    }
    endpoint.messages.lock().unwrap().clone()
}

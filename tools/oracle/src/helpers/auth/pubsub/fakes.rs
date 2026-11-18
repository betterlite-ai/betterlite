use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_auth::auth_pubsub::channel::INVALIDATION_CHANNEL;
use betterlite_auth::auth_pubsub::clock::BackoffClock;
use betterlite_auth::auth_pubsub::decode::decode_invalidation;
use betterlite_auth::auth_pubsub::endpoint::{
    AuthPubSub, AuthPubSubSubscription, PubSubMessage, TransportError,
};
use betterlite_auth::auth_pubsub::eviction::{CacheError, LocalEviction};
use betterlite_auth::auth_pubsub::message::{self, Invalidation};
use serde_json::json;
use tokio::sync::Notify;

use super::{Shared, lock};

#[derive(Clone, Copy)]
pub(crate) enum PublishMode {
    Accept,
    Fail,
    Block,
}

#[derive(Clone)]
pub(crate) enum Step {
    Message(Vec<u8>),
    Fail,
    Block,
}

pub(crate) struct Endpoint {
    pub(crate) journal: Shared,
    sessions: Mutex<VecDeque<Vec<Step>>>,
    publish: PublishMode,
    blocker: Arc<Notify>,
}

impl Endpoint {
    pub(crate) fn new(journal: Shared, sessions: Vec<Vec<Step>>, publish: PublishMode) -> Self {
        Self {
            journal,
            sessions: Mutex::new(VecDeque::from(sessions)),
            publish,
            blocker: Arc::new(Notify::new()),
        }
    }
}

impl AuthPubSub for Endpoint {
    type Subscription = Subscription;

    fn channel(&self) -> String {
        INVALIDATION_CHANNEL.to_owned()
    }

    async fn publish(&self, message: &str) -> Result<(), TransportError> {
        lock(&self.journal)
            .events
            .push("publish-started".to_owned());
        match self.publish {
            PublishMode::Accept => {
                if let Ok(typed) = decode_invalidation(message.as_bytes())
                    && let Some(invalidation) = message::read(&typed)
                {
                    let publication = json!({
                        "cache_key": invalidation.cache_key,
                        "new_value": invalidation.new_value,
                        "ttl": invalidation.ttl,
                    });
                    lock(&self.journal).publications.push(publication);
                }
                lock(&self.journal).events.push("publish".to_owned());
                Ok(())
            }
            PublishMode::Fail => {
                lock(&self.journal).events.push("publish-failed".to_owned());
                Err(TransportError::Publish {
                    message: "scripted publish failed".to_owned(),
                })
            }
            PublishMode::Block => {
                self.blocker.notified().await;
                Ok(())
            }
        }
    }

    async fn subscribe(&self) -> Result<Self::Subscription, TransportError> {
        lock(&self.journal).events.push("subscribe".to_owned());
        match lock(&self.sessions).pop_front() {
            Some(steps) => Ok(Subscription {
                journal: Arc::clone(&self.journal),
                steps: VecDeque::from(steps),
                blocker: Arc::clone(&self.blocker),
            }),
            None => {
                lock(&self.journal)
                    .events
                    .push("subscribe-failed".to_owned());
                Err(TransportError::Connection {
                    message: "scripted subscribe failed".to_owned(),
                })
            }
        }
    }
}

pub(crate) struct Subscription {
    journal: Shared,
    steps: VecDeque<Step>,
    blocker: Arc<Notify>,
}

impl AuthPubSubSubscription for Subscription {
    async fn next_message(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<PubSubMessage>, TransportError> {
        lock(&self.journal)
            .events
            .push(format!("poll:{}", timeout.as_millis()));
        match self.steps.pop_front() {
            Some(Step::Message(data)) => {
                lock(&self.journal).events.push("message".to_owned());
                Ok(Some(PubSubMessage { data: Some(data) }))
            }
            Some(Step::Fail) => Err(TransportError::Connection {
                message: "scripted consume failed".to_owned(),
            }),
            Some(Step::Block) | None => {
                self.blocker.notified().await;
                Ok(None)
            }
        }
    }

    async fn close(&mut self) -> Result<(), TransportError> {
        lock(&self.journal).events.push("close".to_owned());
        Ok(())
    }
}

pub(crate) struct Clock;

impl BackoffClock for Clock {
    async fn sleep(&self, _duration: Duration) {}
}

pub(crate) struct Eviction {
    pub(crate) journal: Shared,
}

impl LocalEviction for Eviction {
    fn apply(&self, invalidation: Invalidation) -> Result<(), CacheError> {
        lock(&self.journal)
            .events
            .push(format!("delete:{}", invalidation.cache_key));
        Ok(())
    }
}

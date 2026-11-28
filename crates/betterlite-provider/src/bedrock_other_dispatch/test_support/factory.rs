use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_bedrock::bedrock_embedding_io::transport::ClientFactory as EmbeddingClientFactory;
use betterlite_bedrock::bedrock_embedding_io::transport::Timeout;
use betterlite_bedrock::image_call::transport::ClientFactory as ImageClientFactory;
use betterlite_bedrock::rerank::transport::ClientFactory as RerankClientFactory;

use super::client::{FixtureClient, Replies, Reply};
use super::record::Seen;

// Which handler kind the factory was asked for, so a test can prove the leaf's own selection
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Sync,
    Async,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClientCall {
    pub kind: Kind,
    pub provider: Option<String>,
    pub timeout: Option<Timeout>,
}

// B173 builds the client each leaf asks for; the fixture hands back its scripted transport and records the ask
pub struct Factory {
    pub replies: Replies,
    pub calls: Mutex<Vec<ClientCall>>,
    pub seen: Seen,
}

impl Factory {
    #[must_use]
    pub fn queued(replies: &[Reply]) -> Self {
        Self {
            replies: Replies(Arc::new(Mutex::new(
                replies.iter().cloned().collect::<VecDeque<Reply>>(),
            ))),
            calls: Mutex::new(Vec::new()),
            seen: Seen::default(),
        }
    }

    #[must_use]
    pub fn calls(&self) -> Vec<ClientCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn client(&self) -> FixtureClient {
        FixtureClient {
            replies: self.replies.clone(),
            seen: self.seen.clone(),
        }
    }

    fn add(&self, kind: Kind, provider: Option<&str>, timeout: Option<Timeout>) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ClientCall {
                kind,
                provider: provider.map(str::to_owned),
                timeout,
            });
    }
}

impl EmbeddingClientFactory for Factory {
    type Client = FixtureClient;

    fn sync_client(&self, timeout: Option<Timeout>) -> Self::Client {
        self.add(Kind::Sync, None, timeout);
        self.client()
    }

    fn async_client(&self, provider: &str, timeout: Option<Timeout>) -> Self::Client {
        self.add(Kind::Async, Some(provider), timeout);
        self.client()
    }
}

impl RerankClientFactory for Factory {
    type Client = FixtureClient;

    fn sync_client(&self) -> Self::Client {
        self.add(Kind::Sync, None, None);
        self.client()
    }

    fn async_client(&self) -> Self::Client {
        self.add(Kind::Async, None, None);
        self.client()
    }
}

impl ImageClientFactory for Factory {
    type Client = FixtureClient;

    fn sync_client(&self) -> Self::Client {
        self.add(Kind::Sync, None, None);
        self.client()
    }

    fn async_client(&self, _timeout: Option<Duration>) -> Self::Client {
        self.add(Kind::Async, None, None);
        self.client()
    }
}

use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_bedrock::bedrock_embedding_io::transport::ClientFactory as EmbeddingClientFactory;
use betterlite_bedrock::bedrock_embedding_io::transport::Timeout;
use betterlite_bedrock::image_call::transport::ClientFactory as ImageClientFactory;
use betterlite_bedrock::rerank::transport::ClientFactory as RerankClientFactory;

use super::client::{FixtureClient, Replies, Reply};
use super::record::{Recorded, Seen};

// Which handler kind the factory was asked for, so a test can prove the leaf's own selection
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Sync,
    Async,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientCall {
    pub kind: Kind,
    pub provider: Option<String>,
}

// B173 builds the client each leaf asks for; the fixture hands back its scripted transport and records the ask
pub struct Factory {
    pub replies: Replies,
    pub calls: Mutex<Vec<ClientCall>>,
    pub seen: Seen,
}

impl Factory {
    #[must_use]
    pub fn new(reply: Reply) -> Self {
        Self::queued(&[reply])
    }

    #[must_use]
    pub fn queued(replies: &[Reply]) -> Self {
        Self {
            replies: Replies::queue(replies),
            calls: Mutex::new(Vec::new()),
            seen: Seen::default(),
        }
    }

    #[must_use]
    pub fn requests(&self) -> Vec<Recorded> {
        self.seen.requests()
    }

    #[must_use]
    pub fn calls(&self) -> Vec<ClientCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn client(&self) -> FixtureClient {
        FixtureClient::new(self.replies.clone(), self.seen.clone())
    }

    fn add(&self, kind: Kind, provider: Option<&str>) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ClientCall {
                kind,
                provider: provider.map(str::to_owned),
            });
    }
}

impl EmbeddingClientFactory for Factory {
    type Client = FixtureClient;

    fn sync_client(&self, _timeout: Option<Timeout>) -> Self::Client {
        self.add(Kind::Sync, None);
        self.client()
    }

    fn async_client(&self, provider: &str, _timeout: Option<Timeout>) -> Self::Client {
        self.add(Kind::Async, Some(provider));
        self.client()
    }
}

impl RerankClientFactory for Factory {
    type Client = FixtureClient;

    fn sync_client(&self) -> Self::Client {
        self.add(Kind::Sync, None);
        self.client()
    }

    fn async_client(&self) -> Self::Client {
        self.add(Kind::Async, None);
        self.client()
    }
}

impl ImageClientFactory for Factory {
    type Client = FixtureClient;

    fn sync_client(&self) -> Self::Client {
        self.add(Kind::Sync, None);
        self.client()
    }

    fn async_client(&self, _timeout: Option<Duration>) -> Self::Client {
        self.add(Kind::Async, None);
        self.client()
    }
}

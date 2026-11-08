use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use tokio::sync::OnceCell;

use super::error::ClientReuseError;
use crate::redis_client_factories::RedisClient;
use crate::redis_client_factories::pool::PoolConfig;
use crate::redis_live_client::{LiveAsyncClient, LiveClientFactory};

// litellm.caching.llm_caching_handler.LLMClientCache keeps at most 200 clients; the oldest one goes first.
const MAX_ENTRIES: usize = 200;

#[derive(Default)]
struct Entries {
    clients: HashMap<String, Arc<OnceCell<LiveAsyncClient>>>,
    order: VecDeque<String>,
}

impl Entries {
    fn insert_new(&mut self, key: String, client: Arc<OnceCell<LiveAsyncClient>>) {
        self.clients.insert(key.clone(), client);
        self.order.push_back(key);
        while self.clients.len() > MAX_ENTRIES {
            if let Some(oldest) = self.order.pop_front() {
                self.clients.remove(&oldest);
            }
        }
    }
}

#[derive(Default)]
pub struct RedisClientStore {
    entries: Mutex<Entries>,
}

impl RedisClientStore {
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(Entries::default()),
        }
    }

    pub async fn get_or_open(
        &self,
        key: &str,
        factory: &LiveClientFactory,
        client: &RedisClient,
        pool: Option<&PoolConfig>,
    ) -> Result<LiveAsyncClient, ClientReuseError> {
        let known = {
            let mut entries = match self.entries.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            let known = entries.clients.get(key).map(Arc::clone);
            let cell = known.unwrap_or_else(|| {
                let cell = Arc::new(OnceCell::new());
                entries.insert_new(key.to_owned(), Arc::clone(&cell));
                cell
            });
            drop(entries);
            cell
        };
        let opened = known
            .get_or_try_init(|| async { factory.open_async(client, pool).await })
            .await?;
        Ok(opened.clone())
    }

    pub fn remove(&self, key: &str) {
        let mut entries = match self.entries.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        entries.clients.remove(key);
        entries.order.retain(|entry| entry != key);
    }
}

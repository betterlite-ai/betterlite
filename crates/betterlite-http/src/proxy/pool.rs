use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::TlsPolicy;
use thiserror::Error;
use tokio_util::task::TaskTracker;

use crate::openai_mock_transport::{TransportChoice, select_transport};
use crate::proxy::limits::TransportLimits;
use crate::proxy::{ProxiedClient, base_builder};
use crate::tls::{TlsError, apply_policy};

#[derive(Debug, Error)]
pub enum PoolError {
    #[error(
        "the pool holds {capacity} clients, one per configured deployment policy, and has no room for another"
    )]
    Full { capacity: usize },
    #[error("the pool is closed")]
    Closed,
    #[error(transparent)]
    Client(#[from] TlsError),
}

struct Entry {
    tls: TlsPolicy,
    routing: Arc<ProxyPolicy>,
    client: ProxiedClient,
}

// Clients are keyed by the immutable policy they were built with and never by a credential, so each request carries its own authorization.
pub struct ClientPool {
    capacity: usize,
    limits: TransportLimits,
    entries: Mutex<Vec<Entry>>,
    tracker: TaskTracker,
}

#[derive(Debug)]
pub enum ClientChoice {
    Supplied(ProxiedClient),
    Mock,
    Pooled(ProxiedClient),
}

impl ClientPool {
    #[must_use]
    pub fn new(capacity: usize, limits: TransportLimits) -> Self {
        Self {
            capacity,
            limits,
            entries: Mutex::new(Vec::new()),
            tracker: TaskTracker::new(),
        }
    }

    pub fn client(
        &self,
        tls: &TlsPolicy,
        routing: &Arc<ProxyPolicy>,
    ) -> Result<ProxiedClient, PoolError> {
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        let found = entries
            .iter()
            .find(|entry| entry.tls == *tls && Arc::ptr_eq(&entry.routing, routing))
            .map(|entry| entry.client.clone());
        if let Some(client) = found {
            return Ok(client);
        }
        if self.tracker.is_closed() {
            return Err(PoolError::Closed);
        }
        if entries.len() >= self.capacity {
            return Err(PoolError::Full {
                capacity: self.capacity,
            });
        }
        let builder = apply_policy(base_builder(), tls)?;
        let mut client =
            ProxiedClient::build(builder, routing, self.limits).map_err(TlsError::Http)?;
        client.tracker = self.tracker.clone();
        entries.push(Entry {
            tls: tls.clone(),
            routing: Arc::clone(routing),
            client: client.clone(),
        });
        drop(entries);
        Ok(client)
    }

    pub fn select(
        &self,
        supplied: Option<ProxiedClient>,
        network_mock: bool,
        tls: &TlsPolicy,
        routing: &Arc<ProxyPolicy>,
    ) -> Result<ClientChoice, PoolError> {
        match select_transport(supplied, network_mock, None::<()>) {
            TransportChoice::Supplied(client) => Ok(ClientChoice::Supplied(client)),
            TransportChoice::Mock => Ok(ClientChoice::Mock),
            TransportChoice::Default => self.client(tls, routing).map(ClientChoice::Pooled),
        }
    }

    pub fn client_count(&self) -> usize {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

mod tests;

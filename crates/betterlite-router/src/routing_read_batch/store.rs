use std::future::Future;
use std::pin::Pin;

use crate::routing_state_store::RoutingStateStore;
use crate::routing_state_store::error::{Observed, StateStoreError};
use crate::routing_value_store::{RouteKey, RouteValue, StoreRead};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReadWarning {
    Unavailable,
    CircuitOpen,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BatchRead<V> {
    pub values: Vec<Option<V>>,
    pub warning: Option<ReadWarning>,
}

pub type BatchReadFuture<'store, V> = Pin<Box<dyn Future<Output = BatchRead<V>> + Send + 'store>>;

pub trait BatchReadStore {
    type Value: Clone + PartialEq + Send + Sync;

    fn batch_read<'store>(
        &'store self,
        keys: &'store [RouteKey],
    ) -> BatchReadFuture<'store, Self::Value>;
}

impl BatchReadStore for RoutingStateStore {
    type Value = RouteValue;

    fn batch_read<'store>(
        &'store self,
        keys: &'store [RouteKey],
    ) -> BatchReadFuture<'store, RouteValue> {
        Box::pin(async move {
            match self.batch_get(keys).await {
                Ok(Observed {
                    value: reads,
                    warning,
                }) => BatchRead {
                    values: reads
                        .into_iter()
                        .map(|read| match read {
                            StoreRead::Missing => None,
                            StoreRead::Value(stored) => Some(stored),
                        })
                        .collect(),
                    warning: warning.map(|_| ReadWarning::Unavailable),
                },
                Err(error) => BatchRead {
                    values: vec![None; keys.len()],
                    warning: Some(match error {
                        StateStoreError::CircuitOpen { .. } => ReadWarning::CircuitOpen,
                        StateStoreError::Settings(_)
                        | StateStoreError::Factory(_)
                        | StateStoreError::Reuse(_)
                        | StateStoreError::Redis { .. }
                        | StateStoreError::Counters { .. }
                        | StateStoreError::Store(_)
                        | StateStoreError::Reply { .. }
                        | StateStoreError::InvalidTtl { .. }
                        | StateStoreError::Conflict { .. }
                        | StateStoreError::UnsupportedTarget
                        | StateStoreError::NoConnection
                        | StateStoreError::NotConfigured => ReadWarning::Unavailable,
                    }),
                },
            }
        })
    }
}

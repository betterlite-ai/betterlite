use std::future::Future;
use std::pin::Pin;

use crate::routing_state_store::RoutingStateStore;
use crate::routing_state_store::error::{Observed, StateStoreError, StoreWarning};
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
                    // a breaker event on a completed read means its half-open probe succeeded, so the values are usable
                    warning: match warning {
                        Some(StoreWarning::Unavailable { .. }) => Some(ReadWarning::Unavailable),
                        Some(StoreWarning::Breaker(_)) | None => None,
                    },
                },
                Err(error) => BatchRead {
                    values: vec![None; keys.len()],
                    warning: Some(if matches!(error, StateStoreError::CircuitOpen { .. }) {
                        ReadWarning::CircuitOpen
                    } else {
                        ReadWarning::Unavailable
                    }),
                },
            }
        })
    }
}

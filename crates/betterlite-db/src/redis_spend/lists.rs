use std::sync::Arc;

use betterlite_redis::cache_primitives::protocol::RedisScalar;
use betterlite_redis::list_operations::RedisPipelineLpopOperation;
use betterlite_redis::list_operations::error::RedisListOperationsError;
use betterlite_redis::list_operations::results::{LpopPipelineEntry, LpopResult};
use betterlite_redis::redis_cache::RedisCache;
use futures_util::future::BoxFuture;

// One popped queue entry as the buffer reads it: the key held nothing, one value, or a list of values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BufferReply {
    Missing,
    Scalar(Vec<u8>),
    List(Vec<Vec<u8>>),
}

impl BufferReply {
    // A scalar reply is the single-entry list of the parser, while a missing one has no entries at all.
    #[must_use]
    pub fn items(self) -> Vec<Vec<u8>> {
        match self {
            Self::Missing => Vec::new(),
            Self::Scalar(value) => vec![value],
            Self::List(values) => values,
        }
    }
}

// One destructive pop of the pipeline: the queue key and at most how many entries to take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BufferPop {
    pub key: String,
    pub count: Option<i64>,
}

#[derive(Debug, thiserror::Error)]
pub enum SpendBufferListError {
    #[error("the redis spend buffer {operation} failed")]
    Redis {
        operation: &'static str,
        #[source]
        source: RedisListOperationsError,
    },
}

pub type SpendListFuture<'call, Value> = BoxFuture<'call, Result<Value, SpendBufferListError>>;

// The three destructive list calls the drain needs, behind a port so tests and the oracle script replies without a server.
pub trait SpendBufferLists: Send + Sync {
    fn pop<'call>(
        &'call self,
        key: &'call str,
        count: Option<i64>,
    ) -> SpendListFuture<'call, BufferReply>;

    // Replies keep one result per operation, and a single value stays distinguishable from a list of them.
    fn pop_pipeline<'call>(
        &'call self,
        operations: &'call [BufferPop],
    ) -> SpendListFuture<'call, Vec<BufferReply>>;

    fn push_and_trim<'call>(
        &'call self,
        key: &'call str,
        values: &'call [String],
        max_len: i64,
    ) -> SpendListFuture<'call, i64>;
}

pub struct RedisSpendLists {
    pub cache: Arc<RedisCache>,
}

impl SpendBufferLists for RedisSpendLists {
    // A live list needs a running Redis, which the db tests fake at the SpendBufferLists seam.
    #[cfg_attr(test, mutants::skip)]
    fn pop<'call>(
        &'call self,
        key: &'call str,
        count: Option<i64>,
    ) -> SpendListFuture<'call, BufferReply> {
        Box::pin(async move {
            let reply = self
                .cache
                .lists
                .async_lpop(key, count)
                .await
                .map_err(|source| SpendBufferListError::Redis {
                    operation: "async_lpop",
                    source,
                })?;
            Ok(pop_reply(reply))
        })
    }

    // A live list needs a running Redis, which the db tests fake at the SpendBufferLists seam.
    #[cfg_attr(test, mutants::skip)]
    fn pop_pipeline<'call>(
        &'call self,
        operations: &'call [BufferPop],
    ) -> SpendListFuture<'call, Vec<BufferReply>> {
        Box::pin(async move {
            let lpop_list: Vec<RedisPipelineLpopOperation> = operations
                .iter()
                .map(|operation| RedisPipelineLpopOperation {
                    key: operation.key.clone(),
                    count: operation.count,
                })
                .collect();
            let entries = self
                .cache
                .lists
                .async_lpop_pipeline(&lpop_list)
                .await
                .map_err(|source| SpendBufferListError::Redis {
                    operation: "async_lpop_pipeline",
                    source,
                })?;
            Ok(entries.into_iter().map(pipeline_reply).collect())
        })
    }

    // A live list needs a running Redis, which the db tests fake at the SpendBufferLists seam.
    #[cfg_attr(test, mutants::skip)]
    fn push_and_trim<'call>(
        &'call self,
        key: &'call str,
        values: &'call [String],
        max_len: i64,
    ) -> SpendListFuture<'call, i64> {
        Box::pin(async move {
            let pushed: Vec<RedisScalar> = values
                .iter()
                .map(|value| RedisScalar::Text(value.clone()))
                .collect();
            self.cache
                .lists
                .async_rpush_and_trim(key, &pushed, max_len)
                .await
                .map_err(|source| SpendBufferListError::Redis {
                    operation: "async_rpush_and_trim",
                    source,
                })
        })
    }
}

// Redis answers a single pop with one value, a list, or nothing; bytes stay bytes when the text is not UTF-8.
#[cfg_attr(test, mutants::skip)]
fn pop_reply(reply: LpopResult) -> BufferReply {
    match reply {
        LpopResult::Missing => BufferReply::Missing,
        LpopResult::Item(text) => BufferReply::Scalar(text.into_bytes()),
        LpopResult::ItemBytes(bytes) => BufferReply::Scalar(bytes),
        LpopResult::Items(texts) => {
            BufferReply::List(texts.into_iter().map(String::into_bytes).collect())
        }
        LpopResult::ItemsBytes(values) => BufferReply::List(values),
    }
}

// The pipeline helper folds an empty array into a missing entry, and a single pop reports the same shapes.
#[cfg_attr(test, mutants::skip)]
fn pipeline_reply(entry: LpopPipelineEntry) -> BufferReply {
    match entry {
        LpopPipelineEntry::Missing => BufferReply::Missing,
        LpopPipelineEntry::Items(texts) => {
            BufferReply::List(texts.into_iter().map(String::into_bytes).collect())
        }
        LpopPipelineEntry::ItemsBytes(values) => BufferReply::List(values),
    }
}

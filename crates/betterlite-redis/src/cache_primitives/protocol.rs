mod tests;

use std::future::Future;

/// `str | bytes | float` as `LiteLLM` passes them to `rpush` and `eval`.
#[derive(Debug, Clone, PartialEq)]
pub enum RedisScalar {
    Text(String),
    Bytes(Vec<u8>),
    Float(f64),
}

/// The async redis commands this cache issues, typed once so Redis and `RedisCluster` clients
/// can share call sites without redis-py's stubs, which omit these methods on `RedisCluster`.
///
/// Methods spell out `impl Future<...> + Send` rather than `async fn` so the trait keeps its
/// `Send` bound on every implementor's future, which an `async fn` in a public trait cannot name.
pub trait AsyncRedisCommands {
    type Error;
    type Reply;
    type Pipeline;

    fn ping(&self) -> impl Future<Output = Result<bool, Self::Error>> + Send;
    fn delete(&self, names: &[&str]) -> impl Future<Output = Result<i64, Self::Error>> + Send;
    fn ttl(&self, name: &str) -> impl Future<Output = Result<i64, Self::Error>> + Send;
    fn expire(
        &self,
        name: &str,
        time: i64,
    ) -> impl Future<Output = Result<bool, Self::Error>> + Send;
    fn rpush(
        &self,
        name: &str,
        values: &[RedisScalar],
    ) -> impl Future<Output = Result<i64, Self::Error>> + Send;
    fn lpop(
        &self,
        name: &str,
        count: Option<i64>,
    ) -> impl Future<Output = Result<Self::Reply, Self::Error>> + Send;
    fn pipeline(&self, transaction: bool) -> Self::Pipeline;
    fn eval(
        &self,
        script: &str,
        numkeys: i64,
        keys_and_args: &[RedisScalar],
    ) -> impl Future<Output = Result<Self::Reply, Self::Error>> + Send;
}

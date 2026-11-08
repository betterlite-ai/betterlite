use std::sync::Arc;

use tokio::runtime::Handle;

use super::connection_info;
use super::credentials::{self, CredentialNeeds as _, RedisCredentialSource, SourceProvider};
use super::plans::{ConnectionPlan, StandalonePlan, plan_cluster, plan_standalone, plan_url};
use super::sentinel::{self, SentinelConnection};
use super::{LiveAsyncClient, LiveClientError, LiveClientFactory, LiveEndpoint};
use crate::redis_client_factories::RedisClient;
use crate::redis_client_factories::pool::PoolConfig;

impl LiveClientFactory {
    #[must_use]
    pub fn new(credentials: Option<Arc<dyn RedisCredentialSource>>) -> Self {
        Self {
            credentials: credentials.map(SourceProvider::from),
        }
    }

    pub async fn open_async(
        &self,
        client: &RedisClient,
        pool: Option<&PoolConfig>,
    ) -> Result<LiveAsyncClient, LiveClientError> {
        let injected = self.credentials.as_ref();
        let pool_limit = pool.map(|config| config.max_connections);
        match client {
            RedisClient::Direct(fields) => {
                let source = credentials::required(injected, fields.needs_source())?;
                let limit = connection_info::concurrency_limit(fields.max_connections, pool_limit)?;
                open_standalone(fields.clone(), source, limit, plan_standalone).await
            }
            RedisClient::Url(record) => {
                let source = credentials::required(injected, record.needs_source())?;
                let limit = connection_info::concurrency_limit(record.max_connections, pool_limit)?;
                open_standalone(record.clone(), source, limit, plan_url).await
            }
            RedisClient::Cluster(record) => {
                let source = credentials::required(injected, record.needs_source())?;
                let limit = connection_info::concurrency_limit(record.max_connections, pool_limit)?;
                let owned = record.clone();
                let plan = Handle::current()
                    .spawn_blocking(move || plan_cluster(&owned, source, limit))
                    .await??;
                let connection = plan.client(None)?.get_async_connection().await?;
                Ok(LiveAsyncClient::Cluster(connection))
            }
            RedisClient::Sentinel(record) => {
                let needed = record.connection_kwargs.credential_provider
                    || record
                        .connection_kwargs
                        .credential_provider_settings
                        .is_some();
                let source = credentials::required(injected, needed)?;
                Ok(LiveAsyncClient::Sentinel(
                    SentinelConnection::open(record.clone(), source).await?,
                ))
            }
        }
    }

    pub fn blocking_endpoint(&self, client: &RedisClient) -> Result<LiveEndpoint, LiveClientError> {
        let injected = self.credentials.as_ref();
        match client {
            RedisClient::Direct(fields) => {
                let source = credentials::required(injected, fields.needs_source())?;
                let limit = connection_info::concurrency_limit(fields.max_connections, None)?;
                endpoint_from(&plan_standalone(fields, source, limit)?)
            }
            RedisClient::Url(record) => {
                let source = credentials::required(injected, record.needs_source())?;
                let limit = connection_info::concurrency_limit(record.max_connections, None)?;
                endpoint_from(&plan_url(record, source, limit)?)
            }
            RedisClient::Cluster(record) => {
                let source = credentials::required(injected, record.needs_source())?;
                let limit = connection_info::concurrency_limit(record.max_connections, None)?;
                endpoint_from(&plan_cluster(record, source, limit)?)
            }
            RedisClient::Sentinel(record) => sentinel::blocking_endpoint(record, injected),
        }
    }
}

fn endpoint_from<P>(plan: &P) -> Result<LiveEndpoint, LiveClientError>
where
    P: ConnectionPlan,
{
    let fetched = credentials::fetch(plan.source())?;
    let connection = plan.client(fetched.as_ref())?;
    Ok(LiveEndpoint {
        target: P::target(connection),
        connect_timeout: plan.timeouts().connect,
        response_timeout: plan.timeouts().response,
    })
}

async fn open_standalone<T, P>(
    record: T,
    source: Option<SourceProvider>,
    limit: usize,
    plan: P,
) -> Result<LiveAsyncClient, LiveClientError>
where
    T: Send + 'static,
    P: FnOnce(&T, Option<SourceProvider>, usize) -> Result<StandalonePlan, LiveClientError>
        + Send
        + 'static,
{
    let planned = Handle::current()
        .spawn_blocking(move || plan(&record, source, limit))
        .await??;
    let connection = planned.client(None)?;
    let manager = connection
        .get_connection_manager_with_config(planned.manager_config())
        .await?;
    Ok(LiveAsyncClient::Standalone(manager))
}

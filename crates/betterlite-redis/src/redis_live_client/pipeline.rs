use futures_util::future::join_all;
use redis::cluster_async::ClusterConnection;
use redis::{Pipeline, RedisResult, Value};

use super::{LiveAsyncClient, LiveClientError};

pub struct LivePipeline {
    pub commands: Pipeline,
    pub client: LiveAsyncClient,
}

impl LivePipeline {
    pub(crate) fn new(client: LiveAsyncClient, transaction: bool) -> Self {
        let mut commands = Pipeline::new();
        if transaction {
            commands.atomic();
        }
        Self { commands, client }
    }

    pub async fn execute(self) -> Result<Vec<Value>, LiveClientError> {
        let results = self.execute_settled().await?;
        let mut values = Vec::with_capacity(results.len());
        let mut first_error = None;
        for result in results {
            match result {
                Ok(value) => values.push(value),
                Err(error) => first_error = first_error.or(Some(error)),
            }
        }
        first_error.map_or_else(|| Ok(values), Err)
    }

    pub async fn execute_settled(
        mut self,
    ) -> Result<Vec<Result<Value, LiveClientError>>, LiveClientError> {
        match &self.client {
            LiveAsyncClient::Cluster(cluster) if !self.commands.is_transaction() => {
                Ok(self.execute_each(cluster).await)
            }
            LiveAsyncClient::Sentinel(sentinel) => {
                self.commands.ignore_errors();
                let mut connection = sentinel.connection().await?;
                match self
                    .commands
                    .query_async::<Vec<RedisResult<Value>>>(&mut connection)
                    .await
                {
                    Ok(results) => Ok(settled(results)),
                    Err(error) => {
                        sentinel.reset_if_stale(&error);
                        Err(LiveClientError::Redis(error))
                    }
                }
            }
            LiveAsyncClient::Standalone(manager) => {
                self.commands.ignore_errors();
                let mut connection = manager.clone();
                Ok(settled(self.commands.query_async(&mut connection).await?))
            }
            LiveAsyncClient::Cluster(cluster) => {
                self.commands.ignore_errors();
                let mut connection = cluster.clone();
                Ok(settled(self.commands.query_async(&mut connection).await?))
            }
        }
    }

    async fn execute_each(
        &self,
        cluster: &ClusterConnection,
    ) -> Vec<Result<Value, LiveClientError>> {
        join_all(self.commands.cmd_iter().map(|command| {
            let owned = command.clone();
            let mut connection = cluster.clone();
            async move {
                owned
                    .query_async::<Value>(&mut connection)
                    .await
                    .map_err(LiveClientError::Redis)
            }
        }))
        .await
    }
}

fn settled(results: Vec<RedisResult<Value>>) -> Vec<Result<Value, LiveClientError>> {
    results
        .into_iter()
        .map(|result| result.map_err(LiveClientError::Redis))
        .collect()
}

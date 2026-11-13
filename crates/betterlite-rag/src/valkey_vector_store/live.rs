use std::collections::BTreeMap;

use async_trait::async_trait;
use betterlite_redis::redis_client_factories::RedisClient;
use betterlite_redis::redis_client_factories::url::UrlClient;
use betterlite_redis::redis_live_client::{LiveAsyncClient, LiveClientFactory};
use redis::{Cmd, Value};
use secrecy::SecretString;

use super::error::ValkeyError;
use super::ports::{
    SocketTimeouts, ValkeyConnector, ValkeyDocument, ValkeySearchParams, ValkeySearchPort,
};
use super::query::ValkeyKnnQuery;
use super::wire;

pub struct LiveValkeyConnector {
    pub factory: LiveClientFactory,
}

pub struct LiveValkeySearchPort {
    pub client: LiveAsyncClient,
}

#[async_trait]
impl ValkeyConnector for LiveValkeyConnector {
    async fn connect(
        &self,
        url: &SecretString,
        timeouts: SocketTimeouts,
    ) -> Result<Box<dyn ValkeySearchPort>, ValkeyError> {
        let record = UrlClient {
            url: url.clone(),
            username: None,
            socket_timeout: Some(timeouts.read.as_secs_f64()),
            socket_connect_timeout: Some(timeouts.connect.as_secs_f64()),
            socket_keepalive: None,
            max_connections: None,
            health_check_interval: None,
            credential_provider: false,
            credential_provider_settings: None,
            passthrough: BTreeMap::new(),
        };
        let client = self
            .factory
            .open_async(&RedisClient::Url(record), None)
            .await?;
        Ok(Box::new(LiveValkeySearchPort { client }))
    }
}

#[async_trait]
impl ValkeySearchPort for LiveValkeySearchPort {
    async fn search(
        &self,
        index: &str,
        query: &ValkeyKnnQuery,
        params: &ValkeySearchParams,
    ) -> Result<Vec<ValkeyDocument>, ValkeyError> {
        let command = search_command(index, query, params);
        let reply: Value = self.client.query(&command).await?;
        wire::documents(&reply)
    }
}

fn search_command(index: &str, query: &ValkeyKnnQuery, params: &ValkeySearchParams) -> Cmd {
    let mut command = redis::cmd("FT.SEARCH");
    command.arg(index).arg(&query.expression);
    command.arg("RETURN").arg(query.return_fields.len());
    for field in &query.return_fields {
        command.arg(field);
    }
    command.arg("DIALECT").arg(query.dialect);
    command.arg("LIMIT").arg(query.offset).arg(query.count);
    command.arg("PARAMS").arg(params.len().saturating_mul(2));
    for (name, value) in params {
        command.arg(name).arg(value);
    }
    command
}

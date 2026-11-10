use betterlite_db::database_url_policy::policy::{
    POOL_TIMEOUT_DEFAULT, PoolTimeouts, TimeoutError, build_db_connection_url_params,
    pg_options_with_timeouts,
};
use betterlite_db::database_url_policy::query::{
    AppendQueryParam, IDLE_LIFETIME_DEFAULT, QueryValue, reader_shareable_params,
};
use betterlite_db::database_urls::cli_url::construct_cli_database_url;
use betterlite_db::database_urls::environment::EnvironmentSource;
use betterlite_db::database_urls::error::DatabaseUrlError;

use super::Settings;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryTarget {
    Writer,
    Direct,
    Reader,
}

impl Settings {
    #[must_use]
    pub fn writer_connection_params(&self) -> Vec<(String, AppendQueryParam)> {
        let timeouts = PoolTimeouts {
            pool_timeout: Some(self.effective_pool_timeout()),
            connect_timeout: self.connect_timeout.clone(),
            socket_timeout: self.socket_timeout.clone(),
        };
        build_db_connection_url_params(
            self.pool_limit,
            timeouts,
            self.disable_prepared_statements,
            &self.extra_params,
        )
    }

    fn effective_pool_timeout(&self) -> QueryValue {
        if let Some(timeout) = self.connection_timeout.clone() {
            return timeout;
        }
        self.pool_timeout
            .clone()
            .unwrap_or(QueryValue::Integer(POOL_TIMEOUT_DEFAULT))
    }

    // B2307's allowlist decides which writer params the replica shares; a bool or list extra param keeps its value
    #[must_use]
    pub fn reader_connection_params(&self) -> Vec<(String, AppendQueryParam)> {
        let writer_params = self.writer_connection_params();
        let placeholders: Vec<(String, QueryValue)> = writer_params
            .iter()
            .map(|(key, _)| (key.clone(), QueryValue::Text(String::new())))
            .collect();
        let allowed: Vec<String> = reader_shareable_params(&placeholders)
            .into_iter()
            .map(|(key, _)| key)
            .collect();
        writer_params
            .into_iter()
            .filter(|(key, _)| allowed.iter().any(|name| name == key))
            .collect()
    }

    #[must_use]
    pub fn cli_idle_lifetime_params(&self) -> Vec<(String, QueryValue)> {
        let lifetime = self
            .max_idle_lifetime
            .clone()
            .unwrap_or(QueryValue::Integer(IDLE_LIFETIME_DEFAULT));
        vec![("max_idle_connection_lifetime".to_owned(), lifetime)]
    }

    pub fn options_for(&self, target: QueryTarget, existing: &str) -> Result<String, TimeoutError> {
        match target {
            QueryTarget::Writer | QueryTarget::Reader => {
                pg_options_with_timeouts(existing, self.statement_timeout, self.lock_timeout)
            }
            QueryTarget::Direct => Ok(existing.to_owned()),
        }
    }

    pub fn cli_writer_url<S>(&self, environment: &S) -> Result<Option<String>, DatabaseUrlError>
    where
        S: EnvironmentSource,
    {
        if let Some(url) = &self.config_url {
            return Ok(Some(url.clone()));
        }
        if let Some(url) = &self.endpoints.database_url {
            return Ok(Some(url.clone()));
        }
        construct_cli_database_url(environment)
    }
}

use std::collections::BTreeMap;

use betterlite_db::database_url_policy::policy::{
    PoolTimeouts, TimeoutError, build_db_connection_url_params, pg_options_with_timeouts,
};
use betterlite_db::database_url_policy::query::{
    AppendQueryParam, QueryValue, reader_shareable_params,
};
use betterlite_db::database_urls::cli_url::construct_cli_database_url;
use betterlite_db::database_urls::error::DatabaseUrlError;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::error::SecretLookupError;
use secrecy::{ExposeSecret as _, SecretString};

use super::Settings;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryTarget {
    Writer,
    Direct,
    Reader,
}

#[derive(Debug, thiserror::Error)]
pub enum CliUrlError {
    #[error(transparent)]
    Secret {
        #[from]
        source: SecretLookupError,
    },
    #[error(transparent)]
    Url {
        #[from]
        source: DatabaseUrlError,
    },
}

impl Settings {
    #[must_use]
    pub fn writer_connection_params(&self) -> Vec<(String, AppendQueryParam)> {
        let timeouts = PoolTimeouts {
            pool_timeout: Some(self.pool_timeout.clone()),
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
        vec![(
            "max_idle_connection_lifetime".to_owned(),
            self.idle_lifetime.clone(),
        )]
    }

    pub fn options_for(&self, target: QueryTarget, existing: &str) -> Result<String, TimeoutError> {
        match target {
            QueryTarget::Writer | QueryTarget::Reader => {
                pg_options_with_timeouts(existing, self.statement_timeout, self.lock_timeout)
            }
            QueryTarget::Direct => Ok(existing.to_owned()),
        }
    }

    // LiteLLM calls get_secret_str only for an os.environ reference; a missing reference leaves DATABASE_URL unset
    pub fn cli_writer_url(
        &self,
        environment: &BTreeMap<String, String>,
        secrets: &SecretLookup,
    ) -> Result<Option<SecretString>, CliUrlError> {
        if let Some(raw) = &self.config_url {
            let text = raw.expose_secret();
            if text.starts_with("os.environ/") {
                return Ok(secrets.get_secret_str(environment, text, None)?);
            }
            return Ok(Some(raw.clone()));
        }
        if let Some(url) = environment.get("DATABASE_URL") {
            return Ok(Some(SecretString::from(url.clone())));
        }
        Ok(construct_cli_database_url(environment)?)
    }
}

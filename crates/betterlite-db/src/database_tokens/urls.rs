use std::error::Error;

use secrecy::{ExposeSecret as _, SecretString};

use crate::database_url_policy::query::add_missing_query_params;
use crate::database_urls::error::DatabaseUrlError;
use crate::database_urls::password::{
    build_reader_url as password_reader_url, build_writer_url as password_writer_url, reader_fields,
};
use crate::database_urls::settings::{
    AZURE_POSTGRESQL_AUTH_ENV_VAR, AuthMode, DatabaseUrlSettings, IAM_TOKEN_DB_AUTH_ENV_VAR,
    ReaderFields,
};
use crate::database_urls::tls::tls_params;
use crate::database_urls::update::{ConfigurationUpdate, WriterEnvResult};

use super::endpoint::IAMEndpoint;
use super::provider::AzureProviderFactory;
use super::settings::TokenToggles;
use super::strategy::{self, DatabaseTokenAuth, DatabaseTokenError, TokenProviders};

#[derive(Debug, thiserror::Error)]
pub enum UrlAssemblyError<AzureError>
where
    AzureError: Error + Send + Sync + 'static,
{
    #[error(transparent)]
    Password {
        #[from]
        source: DatabaseUrlError,
    },
    #[error(transparent)]
    Token {
        #[from]
        source: DatabaseTokenError<AzureError>,
    },
}

// writer assembly resolves the token strategy before the pooled marker, and a token URL replaces even a pinned DATABASE_URL
pub fn build_writer_url<Factory>(
    settings: &DatabaseUrlSettings,
    toggles: TokenToggles,
    pooled: bool,
    providers: &TokenProviders<Factory>,
) -> Result<Option<SecretString>, UrlAssemblyError<Factory::Error>>
where
    Factory: AzureProviderFactory,
{
    let auth = strategy::resolve(toggles, providers)?;
    if auth.is_some() && pooled {
        return Ok(None);
    }
    let Some(token_auth) = &auth else {
        return Ok(password_writer_url(settings, AuthMode::Password)?);
    };
    let endpoint = writer_endpoint(settings, token_auth)?;
    let url = token_url(settings, token_auth, &endpoint)?;
    Ok(Some(SecretString::from(url)))
}

// a reader URL needs a configured reader host and no pinned reader URL, and its token mint ignores the pooled writer marker
pub fn build_reader_url<Factory>(
    settings: &DatabaseUrlSettings,
    toggles: TokenToggles,
    providers: &TokenProviders<Factory>,
) -> Result<Option<SecretString>, UrlAssemblyError<Factory::Error>>
where
    Factory: AzureProviderFactory,
{
    if non_empty(settings.database_host_read_replica.as_deref()).is_none() {
        return Ok(None);
    }
    if settings
        .database_url_read_replica
        .as_ref()
        .is_some_and(|url| !url.expose_secret().is_empty())
    {
        return Ok(None);
    }
    let auth = strategy::resolve(toggles, providers)?;
    let Some(token_auth) = &auth else {
        return Ok(password_reader_url(settings, AuthMode::Password)?);
    };
    let Some(fields) = reader_fields(settings) else {
        return Ok(None);
    };
    let endpoint = reader_endpoint(fields, token_auth)?;
    let url = token_url(settings, token_auth, &endpoint)?;
    Ok(Some(SecretString::from(url)))
}

fn reader_endpoint<Factory>(
    fields: ReaderFields,
    auth: &DatabaseTokenAuth<Factory>,
) -> Result<IAMEndpoint, DatabaseTokenError<Factory::Error>>
where
    Factory: AzureProviderFactory,
{
    match (
        non_empty(fields.user.as_deref()),
        non_empty(fields.name.as_deref()),
    ) {
        (Some(user), Some(name)) => Ok(IAMEndpoint {
            host: fields.host,
            port: fields.port,
            user: user.to_owned(),
            name: name.to_owned(),
            schema: fields.schema,
        }),
        (user, name) => Err(DatabaseTokenError::MissingReaderFields {
            env_var: auth.env_var(),
            fields: missing_fields(&[
                ("DATABASE_USER[_READ_REPLICA]", user.is_none()),
                ("DATABASE_NAME[_READ_REPLICA]", name.is_none()),
            ]),
        }),
    }
}

// the updates carry the URL and normalize only enabled token toggles, and only after a successful assembly
pub fn apply_writer_url_to_env<Factory>(
    settings: &DatabaseUrlSettings,
    toggles: TokenToggles,
    pooled: bool,
    providers: &TokenProviders<Factory>,
) -> Result<WriterEnvResult, UrlAssemblyError<Factory::Error>>
where
    Factory: AzureProviderFactory,
{
    let Some(url) = build_writer_url(settings, toggles, pooled, providers)? else {
        return Ok(WriterEnvResult {
            replaced: false,
            updates: Vec::new(),
        });
    };
    let mut updates = vec![ConfigurationUpdate {
        name: "DATABASE_URL".to_owned(),
        value: url,
    }];
    if toggles.iam_token_db_auth {
        updates.push(ConfigurationUpdate {
            name: IAM_TOKEN_DB_AUTH_ENV_VAR.to_owned(),
            value: SecretString::from("True"),
        });
    }
    if toggles.azure_postgresql_auth {
        updates.push(ConfigurationUpdate {
            name: AZURE_POSTGRESQL_AUTH_ENV_VAR.to_owned(),
            value: SecretString::from("True"),
        });
    }
    Ok(WriterEnvResult {
        replaced: true,
        updates,
    })
}

fn writer_endpoint<Factory>(
    settings: &DatabaseUrlSettings,
    auth: &DatabaseTokenAuth<Factory>,
) -> Result<IAMEndpoint, DatabaseTokenError<Factory::Error>>
where
    Factory: AzureProviderFactory,
{
    let host = non_empty(settings.database_host.as_deref());
    let user = non_empty(settings.database_user.as_deref());
    let name = non_empty(settings.database_name.as_deref());
    let schema = non_empty(settings.database_schema.as_deref());
    iam_endpoint(host, &settings.database_port, user, name, schema).ok_or_else(|| {
        DatabaseTokenError::MissingWriterFields {
            env_var: auth.env_var(),
            fields: missing_fields(&[
                ("DATABASE_HOST", host.is_none()),
                ("DATABASE_USER", user.is_none()),
                ("DATABASE_NAME", name.is_none()),
            ]),
            label: auth.label(),
        }
    })
}

fn iam_endpoint(
    host: Option<&str>,
    port: &str,
    user: Option<&str>,
    name: Option<&str>,
    schema: Option<&str>,
) -> Option<IAMEndpoint> {
    Some(IAMEndpoint {
        host: host?.to_owned(),
        port: port.to_owned(),
        user: user?.to_owned(),
        name: name?.to_owned(),
        schema: schema.map(str::to_owned),
    })
}

fn missing_fields(candidates: &[(&'static str, bool)]) -> String {
    let missing: Vec<&str> = candidates
        .iter()
        .filter(|(_, absent)| *absent)
        .map(|(name, _)| *name)
        .collect();
    missing.join(", ")
}

fn token_url<Factory>(
    settings: &DatabaseUrlSettings,
    auth: &DatabaseTokenAuth<Factory>,
    endpoint: &IAMEndpoint,
) -> Result<String, UrlAssemblyError<Factory::Error>>
where
    Factory: AzureProviderFactory,
{
    let token = strategy::mint_database_token(auth, endpoint)?;
    let assembled = endpoint.build_url(token.expose_secret());
    add_missing_query_params(&assembled, &tls_params(settings)).map_err(|source| {
        UrlAssemblyError::Password {
            source: DatabaseUrlError::Url { source },
        }
    })
}

const fn non_empty(value: Option<&str>) -> Option<&str> {
    match value {
        Some(text) if !text.is_empty() => Some(text),
        Some(_) | None => None,
    }
}

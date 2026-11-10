use secrecy::{ExposeSecret as _, SecretString};

use crate::database_url_policy::query::add_missing_query_params;
use crate::database_urls::endpoint::{Endpoint, password_url};
use crate::database_urls::error::DatabaseUrlError;
use crate::database_urls::settings::{AuthMode, DatabaseUrlSettings, ReaderFields};
use crate::database_urls::tls::tls_params;
use crate::database_urls::update::{ConfigurationUpdate, WriterEnvResult};

// The token branch belongs to the caller's strategy; this password module never mints or falls back to a password there
pub fn build_writer_url(
    settings: &DatabaseUrlSettings,
    auth: AuthMode,
) -> Result<Option<SecretString>, DatabaseUrlError> {
    if auth == AuthMode::Token {
        return Ok(None);
    }
    if settings
        .database_url
        .as_ref()
        .is_some_and(|url| !url.expose_secret().is_empty())
    {
        return Ok(None);
    }
    let (Some(host), Some(user), Some(name)) = (
        settings
            .database_host
            .as_deref()
            .filter(|value| !value.is_empty()),
        settings
            .database_user
            .as_deref()
            .filter(|value| !value.is_empty()),
        settings
            .database_name
            .as_deref()
            .filter(|value| !value.is_empty()),
    ) else {
        return Ok(None);
    };
    let assembled = password_url(&Endpoint {
        user,
        password: settings.database_password.as_ref(),
        host,
        port: &settings.database_port,
        name,
        schema: settings.database_schema.as_deref(),
    });
    let updated = add_missing_query_params(&assembled, &tls_params(settings))?;
    Ok(Some(SecretString::from(updated)))
}

// A reader endpoint is a writer endpoint whose discrete fields resolve to the replica; a pinned writer URL never supplies them
pub fn build_reader_url(
    settings: &DatabaseUrlSettings,
    auth: AuthMode,
) -> Result<Option<SecretString>, DatabaseUrlError> {
    let Some(fields) = reader_fields(settings) else {
        return Ok(None);
    };
    if settings
        .database_url_read_replica
        .as_ref()
        .is_some_and(|url| !url.expose_secret().is_empty())
    {
        return Ok(None);
    }
    build_writer_url(&reader_settings(settings, fields), auth)
}

fn reader_settings(settings: &DatabaseUrlSettings, fields: ReaderFields) -> DatabaseUrlSettings {
    let mut reader = settings.clone();
    reader.database_url = None;
    reader.database_host = Some(fields.host);
    reader.database_port = fields.port;
    reader.database_user = fields.user;
    reader.database_name = fields.name;
    reader.database_schema = fields.schema;
    reader.database_password = fields.password;
    reader
}

#[must_use]
pub fn reader_fields(settings: &DatabaseUrlSettings) -> Option<ReaderFields> {
    let host = settings
        .database_host_read_replica
        .clone()
        .filter(|value| !value.is_empty())?;
    let port = settings
        .database_port_read_replica
        .clone()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| settings.database_port.clone());
    let user = settings
        .database_user_read_replica
        .clone()
        .filter(|value| !value.is_empty())
        .or_else(|| settings.database_user.clone());
    let name = settings
        .database_name_read_replica
        .clone()
        .filter(|value| !value.is_empty())
        .or_else(|| settings.database_name.clone());
    let schema = settings
        .database_schema_read_replica
        .clone()
        .filter(|value| !value.is_empty())
        .or_else(|| settings.database_schema.clone());
    let password = settings
        .database_password_read_replica
        .as_ref()
        .filter(|password| !password.expose_secret().is_empty())
        .cloned()
        .or_else(|| settings.database_password.clone());
    Some(ReaderFields {
        host,
        port,
        user,
        name,
        schema,
        password,
    })
}

pub fn apply_writer_url_to_env(
    settings: &DatabaseUrlSettings,
    auth: AuthMode,
) -> Result<WriterEnvResult, DatabaseUrlError> {
    let writer_url = build_writer_url(settings, auth)?;
    let replaced = writer_url.is_some();
    let updates = writer_url
        .map(|url| ConfigurationUpdate {
            name: "DATABASE_URL".to_owned(),
            value: url,
        })
        .into_iter()
        .collect();
    Ok(WriterEnvResult { replaced, updates })
}
